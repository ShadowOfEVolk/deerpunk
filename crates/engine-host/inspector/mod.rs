//! WebSocket inspector (deerpunk-inspector-protocol).
//! Polled non-blocking from [`Host::frame`] — same thread as the server world.

// Not ARCHIVE: one-off `--inspector-port` CLI overrides must not permanently
// redirect the inspector port in the user's saved config.
deerpunk_engine_common::cvar_def!(
    "sv_inspector_port",
    "9000",
    "WebSocket port for deerpunk-inspector (0 = disabled)."
);
deerpunk_engine_common::cvar_def!(
    "sv_inspector_host",
    "127.0.0.1",
    "Bind address for inspector WS (e.g. 127.0.0.1 or 0.0.0.0)."
);

mod client_share;
mod ws;

pub(crate) use client_share::{ClientEntityRow, ClientInspectShare};

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deerpunk_engine_common::components::{PrefabId, TransformComponent};
use deerpunk_engine_common::world::World;
use deerpunk_inspector_protocol as proto;
use serde_json::{json, Value};

use deerpunk_engine_common::resource::ResourceManager;

use crate::system_profiler;
use deerpunk_engine_server::players::PlayerConnections;

use ws::{WsConnection, WsIncoming};

/// Read-only host slice for inspector RPC (avoids borrowing `Host` + `Inspector` together).
pub struct InspectorRpcCtx<'a> {
    pub world: &'a World,
    pub resources: &'a ResourceManager,
    pub time_secs: f32,
    pub players: &'a PlayerConnections,
    pub server_net: &'a deerpunk_engine_server::ServerNet,
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub server_rep: &'a deerpunk_engine_server::network::ServerPrefabReplication,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<&'a deerpunk_engine_client::client_engine::ClientEngine>,
}

const MAX_REQUESTS_PER_FRAME: usize = 64;

/// Prefab id (FNV-1a of classname) from `PrefabId` for an entity row.
pub(crate) fn entity_prefab_id(world: &World, entity: u64) -> Option<u32> {
    let comp = world.component_init::<PrefabId>();
    world.get::<PrefabId>(entity, comp).map(|p| p.id)
}

/// Local position from `TransformComponent` for an entity row.
pub(crate) fn entity_position(world: &World, entity: u64) -> Option<[f32; 3]> {
    let comp = world.component_init::<TransformComponent>();
    world
        .get::<TransformComponent>(entity, comp)
        .map(|t| t.position.to_array())
}

/// FMOD Studio event snapshot for `fmod_events.*` RPCs.
#[derive(Clone, Debug, Default)]
pub struct FmodEventSnap {
    pub path: String,
    pub bank: String,
    pub is_3d: bool,
    pub oneshot: bool,
    pub params: Vec<String>,
}

/// Component-detail request awaiting the client world's answer (one frame hop).
struct PendingGet {
    request_id: Value,
    seq: u64,
    since: Instant,
}

/// Max age of an in-flight `cliententities.get` before it errors out.
const PENDING_GET_TIMEOUT: Duration = Duration::from_secs(2);

struct ClientState {
    socket: WsConnection,
    pending_get: Option<PendingGet>,
}

struct ActiveInspector {
    listener: TcpListener,
    clients: HashMap<u64, ClientState>,
    next_client_id: u64,
    collections: Vec<proto::CollectionDescriptor>,
}

/// Owned by [`crate::Host`].
pub struct Inspector {
    extra_collections: Vec<proto::CollectionDescriptor>,
    active: Option<ActiveInspector>,
    /// Client-world data published by the main thread (server-thread Host only).
    client_share: Option<Arc<ClientInspectShare>>,
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            extra_collections: Vec::new(),
            active: None,
            client_share: None,
        }
    }

    /// Install the client-world share (server-thread Host, from `server_main`).
    pub fn set_client_share(&mut self, share: Arc<ClientInspectShare>) {
        self.client_share = Some(share);
    }

    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }

    fn merged_collections(&self) -> Vec<proto::CollectionDescriptor> {
        let mut cols = proto::default_collections();
        for c in &self.extra_collections {
            if !cols.iter().any(|e| e.id == c.id) {
                cols.push(c.clone());
            }
        }
        cols
    }

    pub fn register_collection(&mut self, desc: proto::CollectionDescriptor) {
        let id = desc.id.clone();
        if let Some(slot) = self.extra_collections.iter_mut().find(|c| c.id == id) {
            *slot = desc.clone();
        } else {
            self.extra_collections.push(desc.clone());
        }
        if let Some(insp) = self.active.as_mut() {
            if let Some(slot) = insp.collections.iter_mut().find(|c| c.id == id) {
                *slot = desc;
            } else {
                insp.collections.push(desc);
            }
        }
    }

    /// Bind WS listener. No-op if `port == 0` or already started.
    pub fn start(&mut self, host: &str, port: u16) {
        if port == 0 || self.active.is_some() {
            return;
        }

        let host = host.trim().trim_matches('"').trim_matches('\'').trim();
        let host = if host.is_empty() { "127.0.0.1" } else { host };
        let addr = format!("{host}:{port}");
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => {
                let _ = l.set_nonblocking(true);
                log::info!("[inspector] listening ws://{addr}");
                l
            }
            Err(e) => {
                log::error!("[inspector] bind {addr}: {e}");
                return;
            }
        };

        self.active = Some(ActiveInspector {
            listener,
            clients: HashMap::new(),
            next_client_id: 1,
            collections: self.merged_collections(),
        });
    }

    pub fn stop(&mut self) {
        if self.active.is_some() {
            self.active = None;
            log::info!("[inspector] stopped");
        }
    }

    pub fn poll(&mut self, ctx: Option<InspectorRpcCtx<'_>>) {
        let Some(insp) = self.active.as_mut() else {
            return;
        };

        loop {
            match insp.listener.accept() {
                Ok((stream, addr)) => match WsConnection::accept(stream) {
                    Ok(ws) => {
                        log::info!("[inspector] client {addr}");
                        let id = insp.next_client_id;
                        insp.next_client_id += 1;
                        insp.clients.insert(
                            id,
                            ClientState {
                                socket: ws,
                                pending_get: None,
                            },
                        );
                    }
                    Err(e) => log::warn!("[inspector] handshake {addr}: {e}"),
                },
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    log::warn!("[inspector] accept: {e}");
                    break;
                }
            }
        }

        // Main thread only rebuilds client rows while a consumer listens.
        if let Some(share) = &self.client_share {
            share.set_want_rows(!insp.clients.is_empty());
        }
        let client_share = self.client_share.clone();

        let mut disconnected = Vec::new();
        let mut responses: Vec<(u64, String)> = Vec::new();
        let mut budget = MAX_REQUESTS_PER_FRAME;

        for (&id, client) in insp.clients.iter_mut() {
            loop {
                if budget == 0 {
                    break;
                }
                match client.socket.read_incoming() {
                    Ok(WsIncoming::Text(text)) => {
                        budget -= 1;
                        match handle_message(
                            text.trim(),
                            ctx.as_ref(),
                            &insp.collections,
                            client_share.as_deref(),
                            &mut client.pending_get,
                        ) {
                            Ok(msgs) => {
                                for m in msgs {
                                    responses.push((id, m));
                                }
                            }
                            Err(e) => log::warn!("[inspector] client {id}: {e}"),
                        }
                    }
                    Ok(WsIncoming::Ping(p)) => {
                        let _ = client.socket.send_pong(&p);
                    }
                    Ok(WsIncoming::Close) => {
                        disconnected.push(id);
                        break;
                    }
                    Ok(WsIncoming::WouldBlock) => break,
                    Err(e) => {
                        log::warn!("[inspector] read {id}: {e}");
                        disconnected.push(id);
                        break;
                    }
                }
            }
        }

        for (id, msg) in responses {
            if let Some(c) = insp.clients.get_mut(&id) {
                if c.socket.send_text(&msg).is_err() {
                    disconnected.push(id);
                }
            }
        }

        // Deliver deferred `cliententities.get` replies (one-frame command hop).
        for (&id, client) in insp.clients.iter_mut() {
            let Some(pending) = client.pending_get.as_ref() else {
                continue;
            };
            let detail = client_share
                .as_ref()
                .and_then(|s| s.poll_detail(pending.seq));
            match detail {
                Some(Value::Object(o)) if o.contains_key("_error") => {
                    let msg = o
                        .get("_error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("client entity not found");
                    if client
                        .socket
                        .send_text(&proto::err(
                            &pending.request_id,
                            proto::ERR_ENTITY_NOT_FOUND,
                            msg,
                        ))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_get = None;
                }
                Some(value) => {
                    if client
                        .socket
                        .send_text(&proto::ok(&pending.request_id, value))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_get = None;
                }
                None if pending.since.elapsed() >= PENDING_GET_TIMEOUT => {
                    if client
                        .socket
                        .send_text(&proto::err(
                            &pending.request_id,
                            proto::ERR_INTERNAL,
                            "client world did not answer detail request",
                        ))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_get = None;
                }
                None => {}
            }
        }

        for id in disconnected {
            insp.clients.remove(&id);
        }
    }
}

fn handle_message(
    line: &str,
    ctx: Option<&InspectorRpcCtx<'_>>,
    collections: &[proto::CollectionDescriptor],
    client_share: Option<&ClientInspectShare>,
    deferred_get: &mut Option<PendingGet>,
) -> Result<Vec<String>, String> {
    if line.is_empty() {
        return Ok(Vec::new());
    }
    let msg: Value = serde_json::from_str(line).map_err(|e| format!("json: {e}"))?;
    let method = msg
        .get("method")
        .and_then(|v| v.as_str())
        .ok_or("missing method")?;
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(json!({}));

    if let Some(resp) = proto::handle_session_rpc(
        method,
        id.as_ref(),
        &params,
        "deerpunk",
        env!("CARGO_PKG_VERSION"),
        collections,
    ) {
        return Ok(vec![resp]);
    }

    if method == proto::RPC_INITIALIZED {
        return Ok(Vec::new());
    }

    let Some(id) = id else {
        return Ok(Vec::new());
    };

    // `cliententities` needs no server world: rows come from the main-thread
    // snapshot, so it also works at the main menu / on dedicated servers.
    if let Some((col_id, action)) = method.rsplit_once('.') {
        if col_id == "cliententities" {
            return client_entities_rpc(action, &id, &params, client_share, deferred_get);
        }
    }

    // World-dependent RPCs need a running server; the session handshake
    // above still works at the main menu so clients can connect early.
    let Some(ctx) = ctx else {
        return Ok(vec![proto::err(
            &id,
            proto::ERR_INTERNAL,
            "world not running (load a map first)",
        )]);
    };

    if let Some((col_id, action)) = method.rsplit_once('.') {
        let known = collections.iter().any(|c| c.id == col_id);
        if known {
            return match action {
                "list" => Ok(vec![proto::ok(&id, collection_list(col_id, ctx))]),
                "get" => Ok(vec![collection_get(col_id, &id, &params, ctx)]),
                _ => Ok(vec![proto::err(
                    &id,
                    proto::ERR_METHOD_NOT_FOUND,
                    format!("unknown method: {method}"),
                )]),
            };
        }
    }

    Ok(vec![proto::err(
        &id,
        proto::ERR_METHOD_NOT_FOUND,
        format!("unknown method: {method}"),
    )])
}

fn players_value(ctx: &InspectorRpcCtx<'_>) -> Value {
    let remote_addrs: std::collections::HashMap<u32, String> =
        ctx.server_net.remote_slot_debug().into_iter().collect();
    let players: Vec<Value> = ctx
        .players
        .list()
        .iter()
        .map(|p| {
            #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
            let net_id = p.entity.and_then(|e| ctx.server_rep.net_id_of(e));
            #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
            let net_id: Option<u16> = None;
            json!({
                "index": p.slot,
                "name": p.name,
                "local": p.local,
                "entity": p.entity,
                "netId": net_id,
                "addr": remote_addrs.get(&p.slot).cloned().unwrap_or_else(|| {
                    if p.local { "local".into() } else { "?".into() }
                }),
            })
        })
        .collect();
    json!({
        "time": ctx.time_secs,
        "count": players.len(),
        "players": players,
        "role": ctx.server_net.hud_stats().role,
        "hosting": ctx.server_net.is_listening(),
        "client": ctx.client_engine.is_some_and(|c| c.network.is_active()),
    })
}

fn fmod_events(ctx: &InspectorRpcCtx<'_>) -> Vec<FmodEventSnap> {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        return ctx
            .client_engine
            .map(|client| {
                client
                    .audio_list_events()
                    .into_iter()
                    .map(|e| FmodEventSnap {
                        path: e.path,
                        bank: e.bank,
                        is_3d: e.is_3d,
                        oneshot: e.oneshot,
                        params: e.params,
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        Vec::new()
    }
}

/// `cliententities.list` / `cliententities.get` served from the main-thread
/// share; `get` defers its reply by one frame (command hop to the main thread).
fn client_entities_rpc(
    action: &str,
    id: &Value,
    params: &Value,
    client_share: Option<&ClientInspectShare>,
    deferred_get: &mut Option<PendingGet>,
) -> Result<Vec<String>, String> {
    match action {
        "list" => {
            let (time, entities) = match client_share.and_then(|s| s.rows()) {
                Some((time, rows)) => (
                    time,
                    rows.iter()
                        .map(|e| {
                            let mut row = json!({
                                "index": e.id,
                                "name": e.name,
                            });
                            let obj = row.as_object_mut().expect("entity row object");
                            if let Some(prefab_id) = e.prefab_id {
                                obj.insert("prefabId".into(), json!(prefab_id));
                            }
                            if let Some(position) = e.position {
                                obj.insert("position".into(), json!(position));
                            }
                            row
                        })
                        .collect::<Vec<Value>>(),
                ),
                // No client world data (dedicated server, or no client yet).
                None => (0.0, Vec::new()),
            };
            Ok(vec![proto::ok(
                id,
                json!({
                    "time": time,
                    "count": entities.len(),
                    "entities": entities,
                }),
            )])
        }
        "get" => {
            let Some(share) = client_share else {
                return Ok(vec![proto::err(
                    id,
                    proto::ERR_INTERNAL,
                    "no client world (client features disabled)",
                )]);
            };
            let Some(index) = params.get("index").and_then(|v| v.as_u64()) else {
                return Ok(vec![proto::err(
                    id,
                    proto::ERR_INVALID_PARAMS,
                    "missing index",
                )]);
            };
            let known = share
                .rows()
                .map(|(_, rows)| rows.iter().any(|e| e.id == index))
                .unwrap_or(false);
            if !known {
                return Ok(vec![proto::err(
                    id,
                    proto::ERR_ENTITY_NOT_FOUND,
                    "entity not found",
                )]);
            }
            let seq = share.request_detail(index);
            *deferred_get = Some(PendingGet {
                request_id: id.clone(),
                seq,
                since: Instant::now(),
            });
            // Reply is sent from a later poll once the main thread answers.
            Ok(Vec::new())
        }
        _ => Ok(vec![proto::err(
            id,
            proto::ERR_METHOD_NOT_FOUND,
            format!("unknown method: cliententities.{action}"),
        )]),
    }
}

fn collection_list(col_id: &str, ctx: &InspectorRpcCtx<'_>) -> Value {
    let time = ctx.time_secs;
    let world = ctx.world;
    match col_id {
        "serverentities" => {
            let _scope = system_profiler::scope("inspector.serverentities.list");
            let entities: Vec<Value> = world
                .inspect_entities()
                .into_iter()
                .map(|e| {
                    let mut row = json!({
                        "index": e.id,
                        "name": e.name,
                    });
                    let obj = row.as_object_mut().expect("entity row object");
                    if let Some(prefab_id) = entity_prefab_id(world, e.id) {
                        obj.insert("prefabId".into(), json!(prefab_id));
                    }
                    if let Some(position) = entity_position(world, e.id) {
                        obj.insert("position".into(), json!(position));
                    }
                    row
                })
                .collect();
            json!({
                "time": time,
                "count": entities.len(),
                "entities": entities,
            })
        }
        "prefabs" => {
            let classnames = deerpunk_engine_common::scene::PrefabManager::list_disk_classnames(
                &ctx.resources.base_dir(),
            );
            let mut seen: HashMap<u32, String> = HashMap::new();
            let mut items: Vec<Value> = Vec::new();
            for classname in classnames {
                let id = deerpunk_engine_common::components::prefab_id_for(&classname).id;
                if let Some(prev) = seen.get(&id) {
                    log::warn!(
                        "[inspector] prefab id collision: '{classname}' and '{prev}' share id {id}"
                    );
                }
                seen.insert(id, classname.clone());
                items.push(json!({ "id": id, "classname": classname }));
            }
            items.sort_by_key(|v| v.get("id").and_then(Value::as_u64).unwrap_or(0));
            json!({
                "time": time,
                "count": items.len(),
                "items": items,
            })
        }
        "assets" => {
            let assets: Vec<Value> = ctx
                .resources
                .list_assets()
                .into_iter()
                .map(|a| {
                    json!({
                        "index": a.id.0,
                        "name": a.uri,
                        "type": a.type_name,
                        "size": a.size,
                        "aliases": a.aliases,
                        "loaded": a.loaded,
                    })
                })
                .collect();
            json!({
                "time": time,
                "count": assets.len(),
                "assets": assets,
            })
        }
        "systems" => {
            let snap = system_profiler::snapshot_for_inspector();
            if snap.get("time").is_none() {
                let mut o = snap.as_object().cloned().unwrap_or_default();
                o.insert("time".into(), json!(time));
                Value::Object(o)
            } else {
                snap
            }
        }
        "fmod_events" => {
            let fmod = fmod_events(ctx);
            let events: Vec<Value> = fmod
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    json!({
                        "index": i,
                        "name": e.path,
                        "bank": e.bank,
                        "is_3d": e.is_3d,
                        "oneshot": e.oneshot,
                        "params": e.params,
                    })
                })
                .collect();
            json!({
                "time": time,
                "count": events.len(),
                "events": events,
            })
        }
        "players" => players_value(ctx),
        "methoddb" => {
            let methods: Vec<Value> = crate::engine_api::engine_api::MethodDB::inventory_entries()
                .into_iter()
                .enumerate()
                .map(|(i, m)| {
                    json!({
                        "index": i,
                        "name": m.name,
                        "flags": m.flags,
                    })
                })
                .collect();
            json!({
                "time": time,
                "count": methods.len(),
                "methods": methods,
            })
        }
        _ => json!({
            "time": time,
            "count": 0,
            "items": [],
        }),
    }
}

pub(crate) fn component_snapshot(
    world: &World,
    entity: u64,
    slug: &str,
) -> proto::ComponentSnapshot {
    if let Some((schema, data)) = world.inspect_component_fields(entity, slug) {
        let schema = if schema.is_null() { None } else { Some(schema) };
        return proto::component_data(slug, schema, data);
    }
    // No registered inspect dump: still surface the component on the entity.
    proto::ComponentSnapshot::Data {
        slug: slug.to_string(),
        schema: None,
        data: json!({ "_unregistered": true }),
    }
}

fn collection_get(col_id: &str, id: &Value, params: &Value, ctx: &InspectorRpcCtx<'_>) -> String {
    let world = ctx.world;
    let index = match params.get("index").and_then(|v| v.as_u64()) {
        Some(i) => i,
        None => {
            return proto::err(id, proto::ERR_INVALID_PARAMS, "missing index");
        }
    };
    match col_id {
        "serverentities" => {
            let Some(ent) = world.inspect_entities().into_iter().find(|e| e.id == index) else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "entity not found");
            };
            // Enumerate EVERY component on the entity (registered ones dump
            // their fields; unregistered ones surface as `_unregistered`).
            let components: Vec<Value> = world
                .inspect_entity_all_components(ent.id)
                .iter()
                .map(|slug| {
                    serde_json::to_value(component_snapshot(world, ent.id, slug))
                        .unwrap_or(Value::Null)
                })
                .collect();
            proto::ok(
                id,
                json!({
                    "index": ent.id,
                    "name": ent.name,
                    "components": components,
                }),
            )
        }
        "fmod_events" => {
            let fmod = fmod_events(ctx);
            let Some(e) = fmod.get(index as usize) else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "fmod event not found");
            };
            proto::ok(
                id,
                json!({
                    "index": index,
                    "name": e.path,
                    "bank": e.bank,
                    "is_3d": e.is_3d,
                    "oneshot": e.oneshot,
                    "params": e.params,
                }),
            )
        }
        "assets" => {
            let Some(a) = ctx
                .resources
                .list_assets()
                .into_iter()
                .find(|e| e.id.0 == index)
            else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "asset not found");
            };
            proto::ok(
                id,
                json!({
                    "index": a.id.0,
                    "name": a.uri,
                    "type": a.type_name,
                    "size": a.size,
                    "aliases": a.aliases,
                    "loaded": a.loaded,
                }),
            )
        }
        "players" => {
            let list = players_value(ctx)
                .get("players")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let Some(p) = list
                .iter()
                .find(|v| v.get("index").and_then(|i| i.as_u64()) == Some(index))
            else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "player not found");
            };
            proto::ok(id, p.clone())
        }
        "methoddb" => {
            let entries = crate::engine_api::engine_api::MethodDB::inventory_entries();
            let Some(m) = entries.get(index as usize) else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "method not found");
            };
            proto::ok(
                id,
                json!({
                    "index": index,
                    "name": m.name,
                    "flags": m.flags,
                }),
            )
        }
        _ => proto::err(
            id,
            proto::ERR_METHOD_NOT_FOUND,
            format!("unknown collection: {col_id}"),
        ),
    }
}
