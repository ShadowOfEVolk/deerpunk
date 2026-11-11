//! WebSocket inspector (deerpunk-inspector-protocol).
//! Polled non-blocking from [`Host::frame`] on the main thread; server-world
//! collections (`serverentities`, `players`) are served from snapshots the
//! sim thread publishes into [`ServerInspectShare`] (request flag → thread
//! builds → later poll replies, 2s cap, latest-wins). `cliententities` reads
//! the client world synchronously — same thread.

// Not ARCHIVE: one-off `--inspector-port` CLI overrides must not permanently
// redirect the inspector port in the user's saved config.
deerpunk_engine_common::cvar_def!(
    "sv_inspector_port",
    "9000",
    "WebSocket inspector port (0 = disabled).",
    deerpunk_engine_common::cvar_def::ARCHIVE
);
deerpunk_engine_common::cvar_def!(
    "sv_inspector_host",
    "127.0.0.1",
    "Bind address for inspector WS (e.g. 127.0.0.1 or 0.0.0.0)."
);

mod server_share;
mod ws;

pub(crate) use server_share::ServerInspectShare;

pub(crate) use ws::{WsConnection, WsIncoming};

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

#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;

use crate::system_profiler;

/// One `cliententities.list` row (same shape as the server-side row).
pub(crate) struct ClientEntityRow {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) prefab_id: Option<u32>,
    pub(crate) position: Option<[f32; 3]>,
}

/// Read-only main-thread slice for inspector RPC (avoids borrowing `Host` +
/// `Inspector` together). Server-world data flows through `server` snapshots.
pub struct InspectorRpcCtx<'a> {
    pub resources: &'a ResourceManager,
    pub time_secs: f32,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<&'a ClientEngine>,
    /// Server-world snapshots published by the sim thread.
    pub server: &'a ServerInspectShare,
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

/// `serverentities`/`players` detail request awaiting the sim thread's answer
/// (deferred share hop; same 2s cap the client rows used).
struct PendingGet {
    request_id: Value,
    seq: u64,
    since: Instant,
    /// ERR message when the reply carries `_error` (e.g. "player not found").
    not_found_msg: &'static str,
}

/// `serverentities`/`players` list request awaiting a fresh snapshot
/// (latest-wins: a second list supersedes the queued request id).
struct PendingList {
    request_id: Value,
    /// Snapshot generation at queue time; the reply waits for a publish
    /// newer than this so it never serves a pre-request snapshot.
    wait_gen: u64,
    since: Instant,
    /// "server world did not answer …" timeout text.
    timeout_what: &'static str,
}

/// Max age of an in-flight deferred request before it errors out.
const PENDING_GET_TIMEOUT: Duration = Duration::from_secs(2);

struct ClientState {
    socket: WsConnection,
    pending_entities_list: Option<PendingList>,
    pending_players_list: Option<PendingList>,
    pending_entities_get: Option<PendingGet>,
    pending_players_get: Option<PendingPlayersGet>,
}

struct PendingPlayersGet {
    request_id: Value,
    index: u64,
    wait_gen: u64,
    since: Instant,
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
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            extra_collections: Vec::new(),
            active: None,
        }
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
                                pending_entities_list: None,
                                pending_players_list: None,
                                pending_entities_get: None,
                                pending_players_get: None,
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

        // Server snapshots come from the sim thread share (request-driven).
        let server_share = ctx.as_ref().map(|c| c.server);

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
                        match handle_message(text.trim(), ctx.as_ref(), &insp.collections, client) {
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

        let Some(share) = server_share else {
            for id in disconnected {
                insp.clients.remove(&id);
            }
            return;
        };

        // Deliver deferred `serverentities` list replies once the sim thread
        // published a fresh snapshot (latest-wins, 2s cap).
        for (&id, client) in insp.clients.iter_mut() {
            let Some(pending) = client.pending_entities_list.as_ref() else {
                continue;
            };
            let fresh = share.entities_snapshot().map(|(g, _)| g > pending.wait_gen);
            match (fresh, share.entities_snapshot()) {
                (Some(true), Some((_, value))) => {
                    if client
                        .socket
                        .send_text(&proto::ok(&pending.request_id, (*value).clone()))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_entities_list = None;
                }
                _ if pending.since.elapsed() >= PENDING_GET_TIMEOUT => {
                    if client
                        .socket
                        .send_text(&proto::err(
                            &pending.request_id,
                            proto::ERR_INTERNAL,
                            pending.timeout_what,
                        ))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_entities_list = None;
                }
                _ => {}
            }
        }

        // Same for `players` list replies.
        for (&id, client) in insp.clients.iter_mut() {
            let Some(pending) = client.pending_players_list.as_ref() else {
                continue;
            };
            let fresh = share.players_snapshot().map(|(g, _)| g > pending.wait_gen);
            match (fresh, share.players_snapshot()) {
                (Some(true), Some((_, value))) => {
                    if client
                        .socket
                        .send_text(&proto::ok(&pending.request_id, (*value).clone()))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_players_list = None;
                }
                _ if pending.since.elapsed() >= PENDING_GET_TIMEOUT => {
                    if client
                        .socket
                        .send_text(&proto::err(
                            &pending.request_id,
                            proto::ERR_INTERNAL,
                            pending.timeout_what,
                        ))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_players_list = None;
                }
                _ => {}
            }
        }

        // Deferred `serverentities` detail replies (single in-flight request).
        for (&id, client) in insp.clients.iter_mut() {
            let Some(pending) = client.pending_entities_get.as_ref() else {
                continue;
            };
            let detail = share.poll_detail(pending.seq);
            match detail {
                Some(Value::Object(o)) if o.contains_key("_error") => {
                    let msg = o
                        .get("_error")
                        .and_then(|v| v.as_str())
                        .unwrap_or(pending.not_found_msg);
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
                    client.pending_entities_get = None;
                }
                Some(value) => {
                    if client
                        .socket
                        .send_text(&proto::ok(&pending.request_id, value))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_entities_get = None;
                }
                None if pending.since.elapsed() >= PENDING_GET_TIMEOUT => {
                    if client
                        .socket
                        .send_text(&proto::err(
                            &pending.request_id,
                            proto::ERR_INTERNAL,
                            "server world did not answer detail request",
                        ))
                        .is_err()
                    {
                        disconnected.push(id);
                    }
                    client.pending_entities_get = None;
                }
                None => {}
            }
        }

        // Deferred `players` get: find the slot in a fresh players snapshot.
        for (&id, client) in insp.clients.iter_mut() {
            let Some(pending) = client.pending_players_get.as_ref() else {
                continue;
            };
            let fresh = share.players_snapshot().map(|(g, _)| g > pending.wait_gen);
            let reply = match (fresh, share.players_snapshot()) {
                (Some(true), Some((_, value))) => {
                    let found = value
                        .get("players")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| {
                            arr.iter().find(|v| {
                                v.get("index").and_then(|i| i.as_u64()) == Some(pending.index)
                            })
                        })
                        .cloned();
                    Some(match found {
                        Some(p) => proto::ok(&pending.request_id, p),
                        None => proto::err(
                            &pending.request_id,
                            proto::ERR_ENTITY_NOT_FOUND,
                            "player not found",
                        ),
                    })
                }
                _ if pending.since.elapsed() >= PENDING_GET_TIMEOUT => Some(proto::err(
                    &pending.request_id,
                    proto::ERR_INTERNAL,
                    "server world did not answer players request",
                )),
                _ => None,
            };
            if let Some(msg) = reply {
                if client.socket.send_text(&msg).is_err() {
                    disconnected.push(id);
                }
                client.pending_players_get = None;
            }
        }

        for id in disconnected {
            insp.clients.remove(&id);
        }
    }
}

/// `serverentities.*` / `players.*` — deferred through [`ServerInspectShare`]
/// (the server world lives on the sim thread). Reply JSON shapes unchanged.
fn server_collection_rpc(
    col_id: &str,
    action: &str,
    id: &Value,
    params: &Value,
    share: &ServerInspectShare,
    client: &mut ClientState,
) -> Result<Vec<String>, String> {
    match action {
        "list" => {
            if col_id == "serverentities" {
                share.request_entities();
                client.pending_entities_list = Some(PendingList {
                    request_id: id.clone(),
                    wait_gen: share.gen_now(),
                    since: Instant::now(),
                    timeout_what: "server world did not answer entities list",
                });
            } else {
                share.request_players();
                client.pending_players_list = Some(PendingList {
                    request_id: id.clone(),
                    wait_gen: share.gen_now(),
                    since: Instant::now(),
                    timeout_what: "server world did not answer players list",
                });
            }
            // Reply is sent from a later poll once the sim thread publishes.
            Ok(Vec::new())
        }
        "get" => {
            let Some(index) = params.get("index").and_then(|v| v.as_u64()) else {
                return Ok(vec![proto::err(
                    id,
                    proto::ERR_INVALID_PARAMS,
                    "missing index",
                )]);
            };
            if col_id == "serverentities" {
                // Ask for a fresh rows snapshot first so repeated gets stay
                // current; the detail itself is a single in-flight request.
                share.request_entities();
                let seq = share.request_detail(index);
                client.pending_entities_get = Some(PendingGet {
                    request_id: id.clone(),
                    seq,
                    since: Instant::now(),
                    not_found_msg: "entity not found",
                });
            } else {
                share.request_players();
                client.pending_players_get = Some(PendingPlayersGet {
                    request_id: id.clone(),
                    index,
                    wait_gen: share.gen_now(),
                    since: Instant::now(),
                });
            }
            Ok(Vec::new())
        }
        _ => Ok(vec![proto::err(
            id,
            proto::ERR_METHOD_NOT_FOUND,
            format!("unknown method: {col_id}.{action}"),
        )]),
    }
}

fn handle_message(
    line: &str,
    ctx: Option<&InspectorRpcCtx<'_>>,
    collections: &[proto::CollectionDescriptor],
    client: &mut ClientState,
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

    let Some((col_id, action)) = method.rsplit_once('.') else {
        return Ok(vec![proto::err(
            &id,
            proto::ERR_METHOD_NOT_FOUND,
            format!("unknown method: {method}"),
        )]);
    };

    // `cliententities` reads the client world through the RPC ctx, so it also
    // works at the main menu (world off). Server-world collections defer
    // through the share (sim thread); they need a ctx to carry the share.
    if col_id == "cliententities" {
        return client_entities_rpc(action, &id, &params, ctx);
    }
    if col_id == "serverentities" || col_id == "players" {
        let Some(ctx) = ctx else {
            return Ok(vec![proto::err(
                &id,
                proto::ERR_INTERNAL,
                "world not running (load a map first)",
            )]);
        };
        return server_collection_rpc(col_id, action, &id, &params, ctx.server, client);
    }

    // World-independent RPCs work at the main menu; world-dependent ones
    // reply with an error until a map is loaded.
    let Some(ctx) = ctx else {
        return Ok(vec![proto::err(
            &id,
            proto::ERR_INTERNAL,
            "world not running (load a map first)",
        )]);
    };

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

    Ok(vec![proto::err(
        &id,
        proto::ERR_METHOD_NOT_FOUND,
        format!("unknown method: {method}"),
    )])
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
        let _ = ctx;
        Vec::new()
    }
}

/// Entity rows as JSON (same shape the server-side list reply uses).
fn rows_value(rows: &[ClientEntityRow]) -> Vec<Value> {
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
        .collect::<Vec<Value>>()
}

/// `cliententities.list` / `cliententities.get`, served synchronously from
/// the client world in the RPC ctx (single host — no deferred hop anymore).
/// JSON reply shapes are unchanged.
fn client_entities_rpc(
    action: &str,
    id: &Value,
    params: &Value,
    ctx: Option<&InspectorRpcCtx<'_>>,
) -> Result<Vec<String>, String> {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        let client = ctx.and_then(|c| c.client_engine);
        match action {
            "list" => {
                let Some(client) = client else {
                    // No client world (client features disabled): immediate
                    // empty list, same shape as before.
                    return Ok(vec![proto::ok(
                        id,
                        json!({ "time": 0.0, "count": 0, "entities": [] }),
                    )]);
                };
                let world = &client.world;
                let rows: Vec<ClientEntityRow> = world
                    .inspect_entities()
                    .into_iter()
                    .map(|e| ClientEntityRow {
                        id: e.id,
                        prefab_id: entity_prefab_id(world, e.id),
                        position: entity_position(world, e.id),
                        name: e.name,
                    })
                    .collect();
                let entities = rows_value(&rows);
                let time = ctx.map(|c| c.time_secs).unwrap_or(0.0);
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
                let Some(client) = client else {
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
                // Fetch the single entity by id instead of scanning the world.
                let world = &client.world;
                let Some(ent) = world.inspect_entity(index) else {
                    return Ok(vec![proto::err(
                        id,
                        proto::ERR_ENTITY_NOT_FOUND,
                        "entity not found",
                    )]);
                };
                // Every component on the entity; unregistered ones surface as
                // `_unregistered` (same shape as the server detail).
                let components: Vec<Value> = world
                    .inspect_entity_all_components(ent.id)
                    .iter()
                    .map(|slug| {
                        serde_json::to_value(component_snapshot(world, ent.id, slug))
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                Ok(vec![proto::ok(
                    id,
                    json!({
                        "index": ent.id,
                        "name": ent.name,
                        "components": components,
                    }),
                )])
            }
            _ => Ok(vec![proto::err(
                id,
                proto::ERR_METHOD_NOT_FOUND,
                format!("unknown method: cliententities.{action}"),
            )]),
        }
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        let _ = (params, ctx);
        match action {
            // No client world without client features: immediate empty list.
            "list" => Ok(vec![proto::ok(
                id,
                json!({ "time": 0.0, "count": 0, "entities": [] }),
            )]),
            "get" => Ok(vec![proto::err(
                id,
                proto::ERR_INTERNAL,
                "no client world (client features disabled)",
            )]),
            _ => Ok(vec![proto::err(
                id,
                proto::ERR_METHOD_NOT_FOUND,
                format!("unknown method: cliententities.{action}"),
            )]),
        }
    }
}

fn collection_list(col_id: &str, ctx: &InspectorRpcCtx<'_>) -> Value {
    let time = ctx.time_secs;
    match col_id {
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
    let index = match params.get("index").and_then(|v| v.as_u64()) {
        Some(i) => i,
        None => {
            return proto::err(id, proto::ERR_INVALID_PARAMS, "missing index");
        }
    };
    match col_id {
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
