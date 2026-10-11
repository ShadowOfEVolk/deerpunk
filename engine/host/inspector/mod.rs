//! WebSocket inspector server (deerpunk-inspector-protocol).
//! Component field values are stubbed until an FFI layout bridge exists.
//! Host can register extra collections (e.g. `assets`).

mod ws;

use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use deerpunk_engine_common::resource::AssetInfo;
use deerpunk_engine_common::world::{InspectEntity, World};
use deerpunk_inspector_protocol as proto;
use serde_json::{Value, json};

use ws::{WsConnection, WsIncoming};

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
    /// Published `systems.list` payload (darkdemo-style timings).
    systems: Value,
    /// Loaded FMOD Studio events (`fmod_events.list`).
    fmod_events: Vec<FmodEventSnap>,
}

struct ClientState {
    socket: WsConnection,
}

struct InspectorInner {
    snapshot: Arc<Mutex<Snapshot>>,
    collections: Arc<Mutex<Vec<proto::CollectionDescriptor>>>,
    running: Arc<Mutex<bool>>,
}

static mut INSPECTOR: *const InspectorInner = std::ptr::null();

/// Extra collections registered before/after `start` (merged with `entities`).
static EXTRA_COLLECTIONS: OnceLock<Mutex<Vec<proto::CollectionDescriptor>>> = OnceLock::new();

fn extra_collections() -> &'static Mutex<Vec<proto::CollectionDescriptor>> {
    EXTRA_COLLECTIONS.get_or_init(|| Mutex::new(Vec::new()))
}

fn inner() -> Option<&'static InspectorInner> {
    unsafe {
        if INSPECTOR.is_null() {
            None
        } else {
            Some(&*INSPECTOR)
        }
    }
}

pub fn is_running() -> bool {
    inner().is_some()
}

/// Register a host-side collection (descriptor shown in `inspector.initialize`).
/// Safe to call before or after [`start`]; live inspector picks up new descriptors.
pub fn register_collection(desc: proto::CollectionDescriptor) {
    let id = desc.id.clone();
    {
        let mut extras = extra_collections().lock().unwrap();
        if let Some(slot) = extras.iter_mut().find(|c| c.id == id) {
            *slot = desc.clone();
        } else {
            extras.push(desc.clone());
        }
    }
    if let Some(inner) = inner() {
        let mut cols = inner.collections.lock().unwrap();
        if let Some(slot) = cols.iter_mut().find(|c| c.id == id) {
            *slot = desc;
        } else {
            cols.push(desc);
        }
    }
}

fn merged_collections() -> Vec<proto::CollectionDescriptor> {
    let mut cols = proto::default_collections();
    let extras = extra_collections().lock().unwrap();
    for c in extras.iter() {
        if !cols.iter().any(|e| e.id == c.id) {
            cols.push(c.clone());
        }
    }
    cols
}

/// Start WS inspector on `host:port`. No-op if port == 0 or already started.
pub fn start(host: &str, port: u16) {
    if port == 0 {
        return;
    }
    if is_running() {
        return;
    }

    let host = host.trim().trim_matches('"').trim_matches('\'').trim();
    let host = if host.is_empty() { "127.0.0.1" } else { host }.to_string();

    let collections = Arc::new(Mutex::new(merged_collections()));
    let inner = Box::new(InspectorInner {
        snapshot: Arc::new(Mutex::new(Snapshot {
            time: 0.0,
            entities: Vec::new(),
            assets: Vec::new(),
            systems: json!({
                "frame_count": 0,
                "period_secs": 0.0,
                "systems": [],
            }),
            fmod_events: Vec::new(),
        })),
        collections: collections.clone(),
        running: Arc::new(Mutex::new(true)),
    });

    let snapshot = inner.snapshot.clone();
    let running = inner.running.clone();

    thread::spawn(move || {
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

        let mut clients: HashMap<u64, ClientState> = HashMap::new();
        let mut next_id: u64 = 1;

        loop {
            if !*running.lock().unwrap() {
                break;
            }

            loop {
                match listener.accept() {
                    Ok((stream, addr)) => match WsConnection::accept(stream) {
                        Ok(ws) => {
                            log::info!("[inspector] client {addr}");
                            clients.insert(next_id, ClientState { socket: ws });
                            next_id += 1;
                        }
                        Err(e) => log::warn!("[inspector] handshake {addr}: {e}"),
                    },
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        log::warn!("[inspector] accept: {e}");
                        break;
                    }
                }
            }

            let mut disconnected = Vec::new();
            let mut responses: Vec<(u64, String)> = Vec::new();

            for (&id, client) in clients.iter_mut() {
                loop {
                    match client.socket.read_incoming() {
                        Ok(WsIncoming::Text(text)) => {
                            match handle_message(text.trim(), &snapshot, &collections) {
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
                if let Some(c) = clients.get_mut(&id) {
                    if c.socket.send_text(&msg).is_err() {
                        disconnected.push(id);
                    }
                }
            }

            for id in disconnected {
                clients.remove(&id);
            }

            thread::sleep(Duration::from_millis(50));
        }

        log::info!("[inspector] thread exit");
    });

    unsafe {
        INSPECTOR = Box::into_raw(inner);
    }
}

pub fn stop() {
    if let Some(inner) = inner() {
        *inner.running.lock().unwrap() = false;
    }
}

/// Refresh entity list from the server world (call each frame / tick).
pub fn sync_world(world: &World, time: f32) {
    let Some(inner) = inner() else {
        return;
    };
    let entities: Vec<EntitySnap> = world
        .inspect_entities()
        .into_iter()
        .map(|e: InspectEntity| EntitySnap {
            id: e.id,
            name: e.name,
            components: e.components,
        })
        .collect();
    let mut snap = inner.snapshot.lock().unwrap();
    snap.time = time;
    snap.entities = entities;
}

/// Refresh asset list from [`ResourceManager`](deerpunk_engine_common::resource::ResourceManager).
pub fn sync_assets(assets: Vec<AssetInfo>) {
    let Some(inner) = inner() else {
        return;
    };
    inner.snapshot.lock().unwrap().assets = assets;
}

/// Refresh systems profiler snapshot (`systems.list`).
pub fn sync_systems(systems: Value) {
    let Some(inner) = inner() else {
        return;
    };
    inner.snapshot.lock().unwrap().systems = systems;
}

/// Refresh FMOD Studio event list (`fmod_events.list`).
pub fn sync_fmod_events(events: Vec<FmodEventSnap>) {
    let Some(inner) = inner() else {
        return;
    };
    inner.snapshot.lock().unwrap().fmod_events = events;
}

fn handle_message(
    line: &str,
    snapshot: &Arc<Mutex<Snapshot>>,
    collections: &Arc<Mutex<Vec<proto::CollectionDescriptor>>>,
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

    let cols = collections.lock().unwrap().clone();
    if let Some(resp) = proto::handle_session_rpc(
        method,
        id.as_ref(),
        &params,
        "deerpunk",
        env!("CARGO_PKG_VERSION"),
        &cols,
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
        let known = cols.iter().any(|c| c.id == col_id);
        if known {
            return match action {
                "list" => Ok(vec![proto::ok(&id, collection_list(col_id, snapshot))]),
                "get" => Ok(vec![collection_get(col_id, &id, &params, snapshot)]),
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

fn collection_list(col_id: &str, snapshot: &Arc<Mutex<Snapshot>>) -> Value {
    let snap = snapshot.lock().unwrap();
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
        _ => json!({
            "time": snap.time,
            "count": 0,
            "items": [],
        }),
    }
}

fn collection_get(
    col_id: &str,
    id: &Value,
    params: &Value,
    snapshot: &Arc<Mutex<Snapshot>>,
) -> String {
    let index = match params.get("index").and_then(|v| v.as_u64()) {
        Some(i) => i,
        None => {
            return proto::err(id, proto::ERR_INVALID_PARAMS, "missing index");
        }
    };
    let snap = snapshot.lock().unwrap();
    match col_id {
        "entities" => {
            let Some(ent) = snap.entities.iter().find(|e| e.id == index) else {
                return proto::err(id, proto::ERR_ENTITY_NOT_FOUND, "entity not found");
            };
            let components: Vec<Value> = ent
                .components
                .iter()
                .map(|slug| serde_json::to_value(proto::stub_component(slug)).unwrap_or(Value::Null))
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
        _ => proto::err(id, proto::ERR_METHOD_NOT_FOUND, format!("unknown collection: {col_id}")),
    }
}
