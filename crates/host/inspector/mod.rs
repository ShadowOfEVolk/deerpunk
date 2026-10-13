//! WebSocket inspector (deerpunk-inspector-protocol).
//! Polled non-blocking from [`Host::frame`] — same thread as the server world.

mod ws;

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;

use deerpunk_engine_common::world::World;
use deerpunk_inspector_protocol as proto;
use serde_json::{Value, json};

use deerpunk_engine_common::resource::ResourceManager;

use crate::players::PlayerConnections;
use crate::system_profiler;

use ws::{WsConnection, WsIncoming};

/// Read-only host slice for inspector RPC (avoids borrowing `Host` + `Inspector` together).
pub struct InspectorRpcCtx<'a> {
    pub world: &'a World,
    pub resources: &'a ResourceManager,
    pub time_secs: f32,
    pub players: &'a PlayerConnections,
    pub net: &'a crate::net::NetTransport,
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub server_rep: &'a deerpunk_engine_server::network::ServerPrefabReplication,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<&'a deerpunk_engine_client::client_engine::ClientEngine>,
}

const MAX_REQUESTS_PER_FRAME: usize = 64;

/// FMOD Studio event snapshot for `fmod_events.*` RPCs.
#[derive(Clone, Debug, Default)]
pub struct FmodEventSnap {
    pub path: String,
    pub bank: String,
    pub is_3d: bool,
    pub oneshot: bool,
    pub params: Vec<String>,
}

struct ClientState {
    socket: WsConnection,
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

    pub fn poll(&mut self, ctx: InspectorRpcCtx<'_>) {
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
                        insp.clients.insert(id, ClientState { socket: ws });
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
                        match handle_message(text.trim(), &ctx, &insp.collections) {
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

        for id in disconnected {
            insp.clients.remove(&id);
        }
    }

}

fn handle_message(
    line: &str,
    ctx: &InspectorRpcCtx<'_>,
    collections: &[proto::CollectionDescriptor],
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
        ctx.net.remote_slot_debug().into_iter().collect();
    let players: Vec<Value> = ctx
        .players
        .list()
        .iter()
        .map(|p| {
            #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
            let net_id = p.entity.and_then(|e| ctx.server_rep.net_id_of(e));
            #[cfg(not(any(
                feature = "dedicated-server",
                feature = "client",
                feature = "full"
            )))]
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
        "role": ctx.net.hud_stats().role,
        "hosting": ctx.net.is_hosting(),
        "client": ctx.net.is_client(),
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

fn collection_list(col_id: &str, ctx: &InspectorRpcCtx<'_>) -> Value {
    let time = ctx.time_secs;
    let world = ctx.world;
    match col_id {
        "entities" => {
            let _scope = system_profiler::scope("inspector.entities.list");
            let entities: Vec<Value> = world
                .inspect_entities()
                .into_iter()
                .map(|e| {
                    json!({
                        "index": e.id,
                        "name": e.name,
                    })
                })
                .collect();
            json!({
                "time": time,
                "count": entities.len(),
                "entities": entities,
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
        "systems" => system_profiler::snapshot_for_inspector(),
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

fn component_snapshot(world: &World, entity: u64, slug: &str) -> proto::ComponentSnapshot {
    if let Some((schema, data)) = world.inspect_component_fields(entity, slug) {
        let schema = if schema.is_null() {
            None
        } else {
            Some(schema)
        };
        return proto::component_data(slug, schema, data);
    }
    proto::stub_component(slug)
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
        "entities" => {
            let Some(ent) = world
                .inspect_entities()
                .into_iter()
                .find(|e| e.id == index)
            else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "entity not found");
            };
            let components: Vec<Value> = ent
                .components
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
            let Some(p) = list.iter().find(|v| v.get("index").and_then(|i| i.as_u64()) == Some(index))
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
        _ => proto::err(id, proto::ERR_METHOD_NOT_FOUND, format!("unknown collection: {col_id}")),
    }
}
