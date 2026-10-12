//! WebSocket inspector (deerpunk-inspector-protocol).
//! Polled non-blocking from [`Host::frame`] — same thread as the server world.

mod ws;

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;

use deerpunk_engine_common::resource::AssetInfo;
use deerpunk_engine_common::world::{InspectEntity, World};
use deerpunk_inspector_protocol as proto;
use serde_json::{Value, json};

use ws::{WsConnection, WsIncoming};

const MAX_REQUESTS_PER_FRAME: usize = 64;

#[derive(Clone)]
struct EntitySnap {
    id: u64,
    name: String,
    components: Vec<String>,
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

struct Snapshot {
    time: f32,
    entities: Vec<EntitySnap>,
    assets: Vec<AssetInfo>,
    systems: Value,
    fmod_events: Vec<FmodEventSnap>,
    players: Value,
    methoddb: Vec<MethodDbSnap>,
}

#[derive(Clone, Debug)]
pub struct MethodDbSnap {
    pub name: String,
    pub flags: u32,
}

struct ClientState {
    socket: WsConnection,
}

struct ActiveInspector {
    listener: TcpListener,
    clients: HashMap<u64, ClientState>,
    next_client_id: u64,
    snapshot: Snapshot,
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
            snapshot: Snapshot {
                time: 0.0,
                entities: Vec::new(),
                assets: Vec::new(),
                systems: json!({
                    "frame_count": 0,
                    "period_secs": 0.0,
                    "systems": [],
                }),
                fmod_events: Vec::new(),
                players: json!({
                    "time": 0.0,
                    "count": 0,
                    "players": [],
                }),
                methoddb: Vec::new(),
            },
            collections: self.merged_collections(),
        });
    }

    pub fn stop(&mut self) {
        if self.active.is_some() {
            self.active = None;
            log::info!("[inspector] stopped");
        }
    }

    pub fn poll(&mut self, world: &World) {
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
                        match handle_message(
                            text.trim(),
                            &insp.snapshot,
                            &insp.collections,
                            world,
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

        for id in disconnected {
            insp.clients.remove(&id);
        }
    }

    pub fn sync_world(&mut self, world: &World, time: f32) {
        let Some(insp) = self.active.as_mut() else {
            return;
        };
        insp.snapshot.entities = world
            .inspect_entities()
            .into_iter()
            .map(|e: InspectEntity| EntitySnap {
                id: e.id,
                name: e.name,
                components: e.components,
            })
            .collect();
        insp.snapshot.time = time;
    }

    pub fn sync_assets(&mut self, assets: Vec<AssetInfo>) {
        if let Some(insp) = self.active.as_mut() {
            insp.snapshot.assets = assets;
        }
    }

    pub fn sync_systems(&mut self, systems: Value) {
        if let Some(insp) = self.active.as_mut() {
            insp.snapshot.systems = systems;
        }
    }

    pub fn sync_fmod_events(&mut self, events: Vec<FmodEventSnap>) {
        if let Some(insp) = self.active.as_mut() {
            insp.snapshot.fmod_events = events;
        }
    }

    pub fn sync_players(&mut self, players: Value) {
        if let Some(insp) = self.active.as_mut() {
            insp.snapshot.players = players;
        }
    }

    pub fn sync_methoddb(&mut self) {
        let Some(insp) = self.active.as_mut() else {
            return;
        };
        insp.snapshot.methoddb = crate::engine_api::engine_api::MethodDB::inventory_entries()
            .into_iter()
            .map(|e| MethodDbSnap {
                name: e.name.to_string(),
                flags: e.flags,
            })
            .collect();
    }
}

fn handle_message(
    line: &str,
    snapshot: &Snapshot,
    collections: &[proto::CollectionDescriptor],
    world: &World,
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
                "list" => Ok(vec![proto::ok(&id, collection_list(col_id, snapshot))]),
                "get" => Ok(vec![collection_get(col_id, &id, &params, snapshot, world)]),
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

fn collection_list(col_id: &str, snap: &Snapshot) -> Value {
    match col_id {
        "entities" => {
            let entities: Vec<Value> = snap
                .entities
                .iter()
                .map(|e| {
                    json!({
                        "index": e.id,
                        "name": e.name,
                    })
                })
                .collect();
            json!({
                "time": snap.time,
                "count": entities.len(),
                "entities": entities,
            })
        }
        "assets" => {
            let assets: Vec<Value> = snap
                .assets
                .iter()
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
                "time": snap.time,
                "count": assets.len(),
                "assets": assets,
            })
        }
        "systems" => snap.systems.clone(),
        "fmod_events" => {
            let events: Vec<Value> = snap
                .fmod_events
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
                "time": snap.time,
                "count": events.len(),
                "events": events,
            })
        }
        "players" => snap.players.clone(),
        "methoddb" => {
            let methods: Vec<Value> = snap
                .methoddb
                .iter()
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
                "time": snap.time,
                "count": methods.len(),
                "methods": methods,
            })
        }
        _ => json!({
            "time": snap.time,
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

fn collection_get(
    col_id: &str,
    id: &Value,
    params: &Value,
    snap: &Snapshot,
    world: &World,
) -> String {
    let index = match params.get("index").and_then(|v| v.as_u64()) {
        Some(i) => i,
        None => {
            return proto::err(id, proto::ERR_INVALID_PARAMS, "missing index");
        }
    };
    match col_id {
        "entities" => {
            let Some(ent) = snap.entities.iter().find(|e| e.id == index) else {
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
            let Some(e) = snap.fmod_events.get(index as usize) else {
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
            let Some(a) = snap.assets.iter().find(|e| e.id.0 == index) else {
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
            let list = snap
                .players
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
            let Some(m) = snap.methoddb.get(index as usize) else {
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
