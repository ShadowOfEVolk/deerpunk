//! Simulation-thread server facade.
//!
//! The single main-thread [`Host`] hands the sim thread this explicit facade:
//! the [`ServerEngine`] it owns, shared `Arc`s (loopback, cvar, inspector
//! share, enabled mirror) and per-side registries (user messages, resources,
//! mesh registry, gamelib schedule). This is NOT a Host: no cmd system, no fs,
//! no gameinfo, no inspector, no client. Console lines and gamelib command
//! registrations route to the main Host over [`ServerToMain`].

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use deerpunk_engine_common::cvar::CvarSystem;
use deerpunk_engine_common::network::protocol::{CLC, SVC};
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};
use deerpunk_engine_common::network::user_message::UserMessageRpc;
use deerpunk_engine_common::network::{
    HostCommand, NetLoopback, UserCmd, UserCmdAckRpc, USERCMD_SIZE,
};
use deerpunk_engine_common::resource::{MeshRegistry, ResourceManager};
use deerpunk_engine_common::world::World;

use crate::cmd::CmdCallback;
use crate::connect_auth;
use crate::engine_api::engine_api::MethodDB;
use crate::inspector::ServerInspectShare;
use crate::lan;
use crate::net;
use crate::schedule::{CallbackSystem, Schedule};
use crate::system_profiler;
use crate::user_messages::UserMessageRegistry;
use crate::{ServerDll, ServerEngine};

/// Messages from the sim thread to the main-thread Host.
pub enum ServerToMain {
    /// Register a main-console PROXY for a server gamelib command: the proxy
    /// forwards the typed line back over `ServerThreadCmd::ExecCmd`, and the
    /// command executes on the sim thread (same shape as the old
    /// `main_cmd_proxy`).
    RegisterProxyCommand(String),
    /// Server DLL finished `init` — main execs configs + starts the inspector.
    ServerDllStarted,
}

/// Everything the server simulation genuinely needs on its thread.
pub struct ServerThreadCtx {
    /// The simulation itself + the loaded server gamelib DLL.
    pub engine: ServerEngine,
    /// Shared in-process CLC/SVC wire queues (no thread required).
    pub loopback: Arc<NetLoopback>,
    /// Shared cvar table (internally locked; same `Arc` the main Host holds).
    pub cvar: Arc<CvarSystem>,
    /// Game dir (the main Host owns the fs; the thread only needs the path).
    pub gamedir: PathBuf,
    /// Per-side user-message registry (ids must match the client side by
    /// registration order; hooks live on the main Host's registry).
    pub um: UserMessageRegistry,
    /// Gamelib systems from `v1:schedule:add_system` (ticked from [`Self::tick`]).
    pub gamelib_schedule: Schedule,
    /// Server-side resource manager. `ResourceManager` is `RefCell`-based
    /// (not `Sync`), so the thread keeps its own loader set like the old
    /// server-thread Host did.
    pub resources: ResourceManager,
    /// Server-side mesh handles for spawn + `Mesh3D` component writes.
    pub mesh_registry: MeshRegistry,
    /// Console lines / command registrations → main Host.
    pub server_to_host: crossbeam_channel::Sender<ServerToMain>,
    /// Mirrored `engine.enabled` so the main thread can read server state
    /// without touching the engine (old server-thread enabled sink).
    pub enabled_flag: Arc<AtomicBool>,
    /// Inspector publisher for server-world collections (rows, players, detail).
    pub inspect_share: Arc<ServerInspectShare>,
    /// Server gamelib console commands, registered thread-side and executed
    /// here (old server-thread CmdSystem semantics).
    pub console_commands: Vec<ConsoleCommand>,
    /// Set once the gamelib started a map; gates inspector publishes so the
    /// no-world case defers/errors instead of replying with empty snapshots.
    pub inspect_world_ready: bool,
    /// Start-map request that arrived before the gamelib loaded.
    pub pending_start_map: Option<(String, i64)>,
    /// Overload-warning throttle state (5s, shared by both warning kinds).
    pub last_overload_warn: Option<Instant>,
}

/// One server gamelib console command (`v1:cmd:register`). Lives in the
/// facade's own registry; invoked from [`ServerThreadCtx::exec_console_line`]
/// on the sim thread.
#[derive(Clone)]
pub(crate) struct ConsoleCommand {
    pub(crate) name: String,
    pub(crate) callback: CmdCallback,
    pub(crate) user: *mut c_void,
}

impl ServerThreadCtx {
    /// One-time setup on the sim thread (old `Host::server_initialize`):
    /// loaders, engine init, server-world component registration.
    pub fn setup(&mut self) {
        deerpunk_engine_common::resource::register_default_loaders(&self.resources, &self.gamedir);
        if let Err(e) = self.engine.initialize(&self.gamedir) {
            log::error!("[host] server_engine.initialize: {e}");
        }
        let world = &self.engine.world;
        Self::register_server_world_components(world);
    }

    fn register_server_world_components(world: &World) {
        use deerpunk_engine_common::components::*;
        use deerpunk_engine_common::physics::Collider;
        deerpunk_engine_common::scene::register_engine_prefab_appliers(world);
        deerpunk_engine_common::scene::register_engine_component_inspect(world);
        world.component_init::<Terrain>();
        world.component_init::<TerrainSplatMaterial>();
        world.component_init::<Worldspawn>();
        world.component_init::<MapKv>();
        world.component_init::<Collider>();
        world.component_init::<ParticleEmitter>();
    }

    /// Load the server gamelib on this thread. `init` runs here with the
    /// facade as the gamelib's ctx and a facade-overridden resolver; configs
    /// exec + inspector start happen on the main thread after
    /// [`ServerToMain::ServerDllStarted`].
    pub fn load_server_dll(&mut self, path: &str) {
        log::info!("[server-thread] loading gamelib: {path}");
        let ctx_ptr = self as *mut ServerThreadCtx as *mut u8;
        let resolver = MethodDB::new_facade().into_resolver();
        match ServerDll::load(path) {
            Ok(server) => {
                unsafe {
                    server.init(ctx_ptr, resolver);
                }
                self.engine.dll = Some(server);
                log::info!("[server-thread] gamelib loaded: {path}");
            }
            Err(e) => {
                log::error!("server DLL load failed: {e}");
                return;
            }
        }
        let _ = self.server_to_host.send(ServerToMain::ServerDllStarted);
        if let Some((world, seed)) = self.pending_start_map.take() {
            self.apply_gamelib_start_map(&world, seed);
        }
    }

    pub fn apply_gamelib_start_map(&mut self, world: &str, seed: i64) {
        let Some(dll) = self.engine.dll.as_ref() else {
            log::error!("[host] StartMap '{world}': server DLL not loaded");
            return;
        };
        unsafe {
            dll.start_map(world, seed);
        }
        // The server world exists now — inspector publishes may answer.
        self.inspect_world_ready = true;
    }

    /// Queue a start map until the gamelib is loaded (old thread behavior).
    pub fn queue_start_map(&mut self, world: String, seed: i64) {
        self.pending_start_map = Some((world, seed));
    }

    /// Apply enable/disable on this thread's engine (listen / dedicated):
    /// enable flag, auto-listen, LAN advertise/teardown.
    pub fn apply_set_server_enabled(&mut self, on: bool) {
        let was = self.engine.enabled;
        self.engine.set_enabled(on);
        if !on {
            self.engine.slots_play.clear();
        }
        if on {
            if !self.engine.network.is_listening() {
                let port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
                let addr = format!("0.0.0.0:{port}")
                    .parse()
                    .unwrap_or_else(|_| ([0, 0, 0, 0], net::DEFAULT_PORT).into());
                match connect_auth::load_or_create_private_key(&self.gamedir) {
                    Ok(key) => {
                        if let Err(e) = self.engine.network.listen(addr, key) {
                            log::error!("[net] auto-listen failed: {e}");
                        } else {
                            self.start_lan_advertiser(port);
                        }
                    }
                    Err(e) => log::error!("[net] netcode.key: {e}"),
                }
            }
        } else {
            if was && self.engine.network.is_listening() {
                self.engine.network.disconnect();
            }
            self.engine.lan_advertiser = None;
            self.engine.lan_map.clear();
        }
        self.enabled_flag
            .store(self.engine.enabled, Ordering::Relaxed);
    }

    /// One fixed server tick. Order matters: loopback drain → net → inspector
    /// publish → gamelib DLL → schedule → sim → replication → usercmd acks.
    /// One fixed server tick. Deliberately NO profiler scopes around the long
    /// phases: system_profiler/puffin zones hold a global lock, and wrapping
    /// `dll.frame` / `engine.frame` stalled every main-thread zone behind it
    /// (client.ecs / client.inspector ~1.4ms). The old server thread scoped
    /// only the schedule — same here, same name.
    pub fn tick(&mut self, dt: f32) {
        self.drain_loopback_client_packets();
        self.tick_net_server(dt);
        // Publish server-world inspector snapshots when the main-thread poll
        // asked for them (same per-tick cadence the old poll had).
        self.pump_server_inspector();
        self.enabled_flag
            .store(self.engine.enabled, Ordering::Relaxed);
        if !self.engine.enabled {
            return;
        }
        self.engine.sim_time_secs += dt as f64;
        let skin = self.cvar.get_f32_or("ph_capsule_skin", 0.5);
        self.engine.physics.set_capsule_skin(skin);
        if let Some(ref dll) = self.engine.dll {
            unsafe {
                dll.frame(dt);
            }
        }
        {
            let _g = system_profiler::scope("gamelib.schedule");
            self.gamelib_schedule.run(dt);
        }
        let _ = self.engine.frame(dt);
        let packets = self.engine.replication_packets();
        if !packets.is_empty() {
            self.deliver_s2c_broadcast(&packets);
        }
        self.deliver_usercmd_acks();
    }

    /// Overload warning shared by per-tick and catch-up checks (5s throttle,
    /// old server-thread loop behavior).
    pub fn warn_throttled(&mut self) -> bool {
        let now = Instant::now();
        let should = match self.last_overload_warn {
            None => true,
            Some(t) => now.duration_since(t).as_secs_f64() >= 5.0,
        };
        if should {
            self.last_overload_warn = Some(now);
        }
        should
    }

    /// Persist nothing (configs live on the main Host); tear the DLL down.
    pub fn shutdown(&mut self) {
        if let Some(dll) = self.engine.dll.as_ref() {
            unsafe {
                dll.shutdown();
            }
        }
        self.engine.dll = None;
        self.engine.set_enabled(false);
        self.enabled_flag.store(false, Ordering::Relaxed);
        self.engine.shutdown();
    }
}

// ── Net + loopback ───────────────────────────────────────────

impl ServerThreadCtx {
    /// GS-style: one match for every CLC the server accepts.
    fn parse_client_message(&mut self, slot: u32, packet: &[u8]) {
        crate::net_packet_log::log_clc_in(&self.cvar, slot, packet);
        let Some(&cmd) = packet.first() else {
            return;
        };
        let Some(clc) = CLC::from_u8(cmd) else {
            log::warn!("[sv] unknown CLC cmd={cmd} from slot={slot}");
            return;
        };
        match clc {
            CLC::Nop => {}
            CLC::Bad => log::warn!("[sv] CLC::Bad from slot={slot}"),
            CLC::UserCmd => {
                let engine = &mut self.engine;
                if engine.enabled && !engine.slots_play.accepts_usercmd(slot) {
                    return;
                }
                let payload = &packet[1..];
                let Some(ucmd) = UserCmd::decode(payload) else {
                    log::warn!("[sv] bad UserCmd len={} from slot={slot}", payload.len());
                    return;
                };
                engine.players.set_usercmd(slot, ucmd);
            }
            CLC::ClientCommand => {
                let engine = &mut self.engine;
                if engine.enabled && !engine.slots_play.accepts_usercmd(slot) {
                    return;
                }
                let payload = &packet[1..];
                let Some(line) =
                    deerpunk_engine_common::network::decode_client_command_payload(payload)
                else {
                    log::warn!("[sv] bad ClientCommand from slot={slot}");
                    return;
                };
                engine
                    .pending_client_commands
                    .push((slot, line.to_string()));
            }
        }
    }

    fn drain_loopback_client_packets(&mut self) {
        let slot = self.engine.players.local_slot().unwrap_or(0);
        for packet in self.loopback.receive_client_packets() {
            self.parse_client_message(slot, &packet);
        }
    }

    /// Server-side net console commands (the main host forwards `listen` /
    /// `net_disconnect` here when a client host owns the console).
    fn drain_pending_net_cmds(&mut self) {
        for cmd in crate::net_console::take_pending() {
            match cmd {
                crate::net_console::PendingNetCmd::Listen(port_opt) => {
                    let port = port_opt.unwrap_or_else(|| {
                        self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16
                    });
                    self.cvar.set_or_create("sv_port", &port.to_string());
                    self.apply_set_server_enabled(true);
                }
                crate::net_console::PendingNetCmd::NetDisconnect => {
                    self.disconnect_all_server_net();
                    self.engine.slots_play.clear();
                    self.engine.lan_advertiser = None;
                    println!("[net] net_disconnect (server)");
                }
                _ => {}
            }
        }
    }

    fn disconnect_all_server_net(&mut self) {
        self.engine.network.disconnect();
        self.enabled_flag
            .store(self.engine.enabled, Ordering::Relaxed);
    }

    /// Server simulation net tick: listen socket + LAN + C2S.
    pub fn tick_net_server(&mut self, dt: f32) {
        crate::particles_net::drain_pending_sv_emits(self);
        self.drain_pending_net_cmds();
        self.tick_lan();
        let slots = self.engine.players.slots();
        let duration = std::time::Duration::from_secs_f32(dt.max(0.0));
        let lan_reply = if self.engine.network.is_listening() {
            Some(self.lan_advertise_state())
        } else {
            None
        };
        let (c2s, need_public) = self
            .engine
            .network
            .poll(duration, &slots, lan_reply.as_ref());
        if let Some(addr) = need_public {
            self.engine.network.apply_need_public_addr(addr);
        }

        let udp_connects = std::mem::take(&mut self.engine.network.pending_player_connect);
        for &slot in &udp_connects {
            self.engine.on_udp_player_connected(slot);
        }
        if !udp_connects.is_empty() {
            self.finish_client_join(&udp_connects);
        }

        let disconnects = std::mem::take(&mut self.engine.network.pending_player_disconnect);
        for slot in disconnects {
            self.engine.on_udp_player_disconnected(slot);
        }

        for (slot, packet) in c2s {
            self.parse_client_message(slot, &packet);
        }
    }

    fn start_lan_advertiser(&mut self, port: u16) {
        match lan::LanAdvertiser::new(port) {
            Ok(adv) => {
                self.engine.lan_advertiser = Some(adv);
                log::info!("[lan] advertising on port {port}");
            }
            Err(e) => {
                self.engine.lan_advertiser = None;
                log::warn!("[lan] advertiser: {e}");
            }
        }
    }

    fn lan_advertise_state(&self) -> lan::LanAdvertiseState {
        let hostname = self
            .cvar
            .get_string("hostname")
            .unwrap_or_else(|| "DarkDemo".into());
        let default_port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
        let port = self.engine.network.listen_port().unwrap_or(default_port);
        let map = if self.engine.lan_map.is_empty() {
            "?".into()
        } else {
            self.engine.lan_map.clone()
        };
        let players = self.engine.players.list().len().min(255) as u8;
        lan::LanAdvertiseState {
            hostname,
            map,
            port,
            players,
            max_players: 16,
        }
    }

    fn tick_lan(&mut self) {
        if self.engine.lan_advertiser.is_some() {
            let state = self.lan_advertise_state();
            if let Some(adv) = self.engine.lan_advertiser.as_mut() {
                adv.tick(&state);
            }
        }
    }
}

// ── S2C delivery + join ──────────────────────────────────────

impl ServerThreadCtx {
    fn local_client_in_play(&self) -> bool {
        let Some(slot) = self.engine.players.local_slot() else {
            return false;
        };
        self.engine.slots_play.accepts_usercmd(slot)
    }

    /// S2C to one slot: UDP or in-process Channel.
    fn deliver_s2c_to_slot(&mut self, slot: u32, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        let is_local = self
            .engine
            .players
            .get(slot)
            .map(|p| p.local)
            .unwrap_or(false);
        let via = if is_local {
            format!("s2c slot={slot} local")
        } else {
            format!("s2c slot={slot} udp")
        };
        for p in packets {
            crate::net_packet_log::log_s2c_out(&self.cvar, &via, p);
        }
        if is_local {
            for p in packets {
                self.loopback.send_server_packet(p.clone());
            }
        } else {
            self.engine.network.send_s2c_to_slot(slot, packets);
        }
    }

    /// Play-phase replication to all clients in play (UDP + local channel).
    pub(crate) fn deliver_s2c_broadcast(&mut self, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        for p in packets {
            crate::net_packet_log::log_s2c_out(&self.cvar, "s2c broadcast", p);
        }
        self.engine.network.broadcast_s2c(packets);
        if self.local_client_in_play() {
            for p in packets {
                self.loopback.send_server_packet(p.clone());
            }
        }
    }

    fn deliver_usercmd_acks(&mut self) {
        let tick = {
            let engine = &mut self.engine;
            engine.sim_tick = engine.sim_tick.wrapping_add(1);
            engine.sim_tick
        };
        let slots = self.engine.players.slots();
        let mut pending: Vec<(u32, Vec<u8>)> = Vec::new();
        {
            let engine = &mut self.engine;
            for slot in slots {
                let Some(p) = engine.players.get(slot) else {
                    continue;
                };
                let seq = p.usercmd.sequence;
                if seq <= p.cmd_ack_sent {
                    continue;
                }
                if let Some(p) = engine.players.get_mut(slot) {
                    p.cmd_ack_sent = seq;
                }
                let rpc = UserCmdAckRpc {
                    ack_sequence: seq,
                    server_tick: tick,
                };
                pending.push((slot, rpc.encode_packet()));
            }
        }
        for (slot, packet) in pending {
            self.deliver_s2c_to_slot(slot, &[packet]);
        }
    }

    /// `player_connected` + catchup + join init (UDP remote or in-process).
    pub(crate) fn finish_client_join(&mut self, slots: &[u32]) {
        if slots.is_empty() {
            return;
        }
        if !self.engine.enabled {
            return;
        }
        for &slot in slots {
            if let Some(ref dll) = self.engine.dll {
                unsafe {
                    dll.player_connected(slot);
                }
            }
        }
        for &slot in slots {
            self.send_join_init(slot);
        }
        let catchup = {
            let engine = &mut self.engine;
            engine.rep.tick(&mut engine.world)
        };
        self.deliver_s2c_broadcast(&catchup);
    }

    /// Join init bundle for one slot (then play via `InitDone`).
    fn send_join_init(&mut self, slot: u32) {
        let engine = &mut self.engine;
        let defs = engine
            .rep
            .prefab_definition_packets(&self.gamedir, &engine.world);
        let snap = engine.rep.snapshot(&engine.world);
        let def_count = defs.len();
        let snap_count = snap.len();
        self.deliver_s2c_to_slot(slot, &defs);
        if let Some(ref dll) = self.engine.dll {
            unsafe {
                dll.join_init(slot, 0);
            }
        }
        self.deliver_s2c_to_slot(slot, &snap);
        if let Some(ref dll) = self.engine.dll {
            unsafe {
                dll.join_init(slot, 1);
            }
        }
        self.deliver_s2c_to_slot(slot, &[vec![SVC::InitDone as u8]]);
        self.engine.slots_play.mark_ready(slot);
        log::info!(
            "[net] init slot={slot}: {def_count} prefab defs, {snap_count} snap pkts → InitDone"
        );
    }

    /// Local listen client joins the running server (channel hop from main).
    pub fn connect_local(&mut self, name: String) {
        if self.engine.players.connect_local(name).is_some() {
            let slot = self.engine.players.local_slot().unwrap_or(0);
            self.finish_client_join(&[slot]);
        }
    }

    /// Send a registered UserMessage (`slot = None` → broadcast remotes).
    pub fn um_send(&mut self, name: &str, data: &[u8], slot: Option<u32>) -> Result<(), String> {
        let Some(msg_id) = self.um.id_of(name) else {
            return Err(format!("user message '{name}' not registered"));
        };
        let rpc = UserMessageRpc::new(msg_id, data.to_vec());
        let pkt = rpc.to_packet();
        match slot {
            None => self.deliver_s2c_broadcast(&[pkt]),
            Some(s) => self.deliver_s2c_to_slot(s, &[pkt]),
        }
        Ok(())
    }
}

// ── Sound + particles ────────────────────────────────────────

impl ServerThreadCtx {
    pub fn alloc_sound_id(&mut self) -> u32 {
        let id = self.engine.next_sound_id;
        self.engine.next_sound_id = self.engine.next_sound_id.wrapping_add(1);
        if self.engine.next_sound_id == 0 {
            self.engine.next_sound_id = 1;
        }
        id
    }

    pub fn queue_sound_packet(&mut self, packet: Vec<u8>) {
        self.loopback.send_server_packet(packet);
    }

    /// Emit a particle burst from the server to all play-phase clients.
    pub fn particles_emit(
        &mut self,
        effect: &str,
        pos: [f32; 3],
        vel: [f32; 3],
        payload: &deerpunk_engine_particles::SpawnPayload,
    ) {
        let rpc = crate::particles_net::ParticlesEmitRpc {
            effect: effect.to_string(),
            pos,
            vel,
            payload: payload.clone(),
        };
        self.deliver_s2c_broadcast(&[rpc.to_packet()]);
    }
}

// ── Inspector publisher ──────────────────────────────────────

impl ServerThreadCtx {
    /// Build server-side inspector snapshots the main-thread poll asked for.
    /// Reply JSON shapes are identical to the old synchronous server path.
    fn pump_server_inspector(&mut self) {
        if !self.inspect_world_ready {
            // No map yet: never publish, so deferred requests hit the 2s cap
            // with an error instead of instant empty snapshots.
            return;
        }
        if self.inspect_share.take_entities_request() {
            let world = &self.engine.world;
            let time = self.engine.sim_time_secs as f32;
            let entities: Vec<serde_json::Value> = world
                .inspect_entities()
                .into_iter()
                .map(|e| {
                    let mut row = serde_json::json!({
                        "index": e.id,
                        "name": e.name,
                    });
                    let obj = row.as_object_mut().expect("entity row object");
                    if let Some(prefab_id) = crate::inspector::entity_prefab_id(world, e.id) {
                        obj.insert("prefabId".into(), serde_json::json!(prefab_id));
                    }
                    if let Some(position) = crate::inspector::entity_position(world, e.id) {
                        obj.insert("position".into(), serde_json::json!(position));
                    }
                    row
                })
                .collect();
            let value = serde_json::json!({
                "time": time,
                "count": entities.len(),
                "entities": entities,
            });
            self.inspect_share.publish_entities(value);
        }
        if self.inspect_share.take_players_request() {
            self.inspect_share.publish_players(self.players_value());
        }
        if let Some((seq, entity)) = self.inspect_share.take_pending_detail() {
            let world = &self.engine.world;
            let value = match world.inspect_entity(entity) {
                None => serde_json::json!({ "_error": "entity not found" }),
                Some(ent) => {
                    // Every component on the entity; unregistered ones surface
                    // as `_unregistered`.
                    let components: Vec<serde_json::Value> = world
                        .inspect_entity_all_components(ent.id)
                        .iter()
                        .map(|slug| {
                            serde_json::to_value(crate::inspector::component_snapshot(
                                world, ent.id, slug,
                            ))
                            .unwrap_or(serde_json::Value::Null)
                        })
                        .collect();
                    serde_json::json!({
                        "index": ent.id,
                        "name": ent.name,
                        "components": components,
                    })
                }
            };
            self.inspect_share.publish_detail(seq, value);
        }
    }

    /// Players snapshot (`players.list` shape; `client` was always `false` on
    /// the sim thread — kept that way).
    fn players_value(&self) -> serde_json::Value {
        use serde_json::json;
        let remote_addrs: std::collections::HashMap<u32, String> = self
            .engine
            .network
            .remote_slot_debug()
            .into_iter()
            .collect();
        let players: Vec<serde_json::Value> = self
            .engine
            .players
            .list()
            .iter()
            .map(|p| {
                let net_id = p.entity.and_then(|e| self.engine.rep.net_id_of(e));
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
            "time": self.engine.sim_time_secs as f32,
            "count": players.len(),
            "players": players,
            "role": self.engine.network.hud_stats().role,
            "hosting": self.engine.network.is_listening(),
            "client": false,
        })
    }
}

// ── Server world helpers ─────────────────────────────────────

impl ServerThreadCtx {
    pub fn clear_server_world(&mut self) {
        let world = &mut self.engine.world;
        world.clear_deferred_destroy();
        let ids: Vec<u64> = world.inspect_entities().into_iter().map(|e| e.id).collect();
        log::info!("[world] clear — destroying {} entities", ids.len());
        for id in ids {
            world.destroy(id);
        }
        world.simulate(0.0);
        log::debug!("[world] clear done");
    }

    pub fn world_get_component_ptr(&mut self, entity: u64, comp: u64) -> *mut c_void {
        let ptr = self.engine.world.get_raw(entity, comp);
        if ptr.is_null() {
            std::ptr::null_mut()
        } else {
            ptr as *mut std::ffi::c_void
        }
    }

    pub fn world_set_component_raw(
        &mut self,
        entity: u64,
        comp: u64,
        data: *const std::ffi::c_void,
        size: usize,
    ) {
        if size == std::mem::size_of::<deerpunk_engine_common::components::mesh3d::Mesh3D>() {
            crate::mesh_components::set_mesh3d_raw(
                &mut self.engine.world,
                &mut self.mesh_registry,
                entity,
                comp,
                data,
                size,
            );
            return;
        }
        if size == std::mem::size_of::<deerpunk_engine_common::components::SkinnedMesh3D>() {
            crate::mesh_components::set_skinned_raw(
                &mut self.engine.world,
                &mut self.mesh_registry,
                entity,
                comp,
                data,
                size,
            );
            return;
        }
        let world = &mut self.engine.world;
        crate::world_host_api::set_component_raw_fallback(world, entity, comp, data, size);
    }

    /// Spawn `{gamedir}/prefabs/{name}.prefab` into the server world.
    pub fn spawn_prefab(
        &mut self,
        name: &str,
        opts: &deerpunk_engine_api::spawn_prefab::SpawnPrefabOpts,
    ) -> Result<u64, String> {
        let map = crate::spawn_prefab::map_spawn_opts_from_ffi(opts)?;
        deerpunk_engine_common::scene::spawn_prefab_entity(
            &mut self.engine.world,
            &mut self.mesh_registry,
            &self.gamedir,
            &self.resources,
            name,
            map.as_ref(),
        )
        .map_err(|e| e.message)
    }

    pub fn register_component_desc(
        &mut self,
        desc: &deerpunk_engine_ffi::ComponentRegisterDesc,
    ) -> u64 {
        let Some(reg) = crate::engine_api::components::build_component_descriptor(desc) else {
            return 0;
        };
        self.engine.world.component_init_raw(&reg)
    }

    /// Rapier segment trace — returns hit entity + fraction/normal/point.
    #[allow(clippy::too_many_arguments)]
    pub fn physics_trace_line(
        &mut self,
        ox: f32,
        oy: f32,
        oz: f32,
        ex: f32,
        ey: f32,
        ez: f32,
        exclude_entity: u64,
        out: &mut crate::physics_trace::PhysicsTraceResult,
    ) -> i32 {
        let origin = [ox, oy, oz];
        let delta = [ex - ox, ey - oy, ez - oz];
        let len = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if len < 1e-6 {
            *out = crate::physics_trace::PhysicsTraceResult {
                hit: 0,
                entity: 0,
                fraction: 1.0,
                point: origin,
                normal: [0.0, 0.0, 1.0],
            };
            return 0;
        }
        let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
        match self
            .engine
            .physics
            .cast_ray(origin, dir, len, exclude_entity)
        {
            Some(h) => {
                *out = crate::physics_trace::PhysicsTraceResult {
                    hit: 1,
                    entity: h.entity,
                    fraction: h.fraction,
                    point: h.point,
                    normal: h.normal,
                };
                1
            }
            None => {
                *out = crate::physics_trace::PhysicsTraceResult {
                    hit: 0,
                    entity: 0,
                    fraction: 1.0,
                    point: [ex, ey, ez],
                    normal: [0.0, 0.0, 1.0],
                };
                0
            }
        }
    }
}

// ── Facade MethodDB binds (server DLL ctx) ───────────────────
//
// These trampolines dereference the gamelib ctx pointer as
// `ServerThreadCtx`. They never touch main-Host state — only the facade's
// fields and the shared Arcs.

use deerpunk_engine_macro as methoddb;

#[methoddb::namespace(name = "v1:world", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "spawn")]
    pub fn api_world_spawn(&mut self) -> u64 {
        self.engine.world.spawn()
    }

    #[methoddb::bind(name = "has")]
    pub fn api_world_has(&self, entity: u64, comp: u64) -> bool {
        self.engine.world.has(entity, comp)
    }

    #[methoddb::bind(name = "remove")]
    pub fn api_world_remove(&mut self, entity: u64, comp: u64) {
        self.engine.world.remove(entity, comp);
    }

    #[methoddb::bind(name = "get")]
    pub fn api_world_get(&mut self, entity: u64, comp: u64) -> *mut c_void {
        self.world_get_component_ptr(entity, comp)
    }

    #[methoddb::bind(name = "set")]
    pub fn api_world_set(&mut self, entity: u64, comp: u64, data: *const c_void, size: usize) {
        self.world_set_component_raw(entity, comp, data, size);
    }

    #[methoddb::bind(name = "clear")]
    pub fn api_world_clear(&mut self) {
        self.clear_server_world();
    }

    #[methoddb::bind(name = "free_value")]
    pub fn api_world_free_value(&mut self, _ptr: *mut c_void) {}

    /// Spawn `{gamedir}/prefabs/{name}.prefab` into the server world.
    #[methoddb::bind(name = "spawn_prefab")]
    pub fn api_world_spawn_prefab(
        &mut self,
        name: &str,
        opts: &deerpunk_engine_api::spawn_prefab::SpawnPrefabOpts,
    ) -> u64 {
        if name.is_empty() {
            return 0;
        }
        match self.spawn_prefab(name, opts) {
            Ok(id) => id,
            Err(e) => {
                log::error!("[world] spawn_prefab {name}: {e}");
                0
            }
        }
    }

    #[methoddb::bind(name = "spawn_bundle")]
    pub fn api_world_spawn_bundle(
        &mut self,
        _comp_ids: *const u64,
        _comp_data: *const *mut c_void,
        _count: usize,
    ) -> u64 {
        0
    }
}

#[methoddb::namespace(name = "v1:world", on_host, ctx = "server", scope = ["server"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "destroy")]
    pub fn api_world_destroy(&mut self, entity: u64) {
        self.engine.world.destroy(entity);
    }

    #[methoddb::bind(name = "destroy_deferred")]
    pub fn api_world_destroy_deferred(&mut self, entity: u64) {
        self.engine.world.destroy_deferred(entity);
    }

    #[methoddb::bind(name = "component_register")]
    pub fn api_world_component_register(
        &mut self,
        desc: &deerpunk_engine_ffi::ComponentRegisterDesc,
    ) -> u64 {
        self.register_component_desc(desc)
    }
}

#[methoddb::namespace(name = "v1:world", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "query")]
    pub fn api_world_query(
        &mut self,
        require: &[u64],
        exclude: &[u64],
        out: *mut u64,
        max: usize,
    ) -> usize {
        let _ = exclude;
        let qid = self.engine.world.query_prepare_ids(require);
        let mut ids = Vec::new();
        if let Some(batches) = self.engine.world.query_batches(qid) {
            for batch in batches {
                let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                ids.extend_from_slice(entities);
            }
        }
        self.engine.world.query_release(qid);
        let mut written = 0usize;
        for &e in &ids {
            if written >= max || out.is_null() {
                break;
            }
            unsafe {
                *out.add(written) = e;
            }
            written += 1;
        }
        written
    }

    #[methoddb::bind(name = "query_count")]
    pub fn api_world_query_count(&mut self, require: &[u64], exclude: &[u64]) -> usize {
        let _ = exclude;
        let qid = self.engine.world.query_prepare_ids(require);
        let mut count = 0usize;
        if let Some(batches) = self.engine.world.query_batches(qid) {
            for batch in batches {
                count += batch.count;
            }
        }
        self.engine.world.query_release(qid);
        count
    }

    #[methoddb::bind(name = "query_matches")]
    pub fn api_world_query_matches(&self, entity: u64, require: &[u64], exclude: &[u64]) -> bool {
        for &c in require {
            if !self.engine.world.has(entity, c) {
                return false;
            }
        }
        for &c in exclude {
            if self.engine.world.has(entity, c) {
                return false;
            }
        }
        true
    }

    #[methoddb::bind(name = "query_prepare")]
    pub fn api_world_query_prepare(&mut self, expr: &str) -> u32 {
        self.engine.world.query_prepare(expr) as u32
    }

    #[methoddb::bind(name = "query_execute")]
    pub fn api_world_query_execute(
        &mut self,
        query_id: u32,
        callback: crate::gamelib_abi::WorldQueryCallback,
        ctx: *mut c_void,
    ) {
        let qid = query_id as usize;
        if let Some(batches) = self.engine.world.query_batches(qid) {
            for batch in batches {
                let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let field_count = batch.columns.len();
                for (i, &entity) in entities.iter().enumerate() {
                    let fields: Vec<*const std::ffi::c_void> = (0..field_count)
                        .map(|f| unsafe { batch.columns[f].add(i * batch.sizes[f]) })
                        .collect();
                    callback(entity, fields.as_ptr(), field_count, ctx);
                }
            }
        }
    }

    #[methoddb::bind(name = "system_register")]
    pub fn api_world_system_register(
        &mut self,
        name: &str,
        query: &str,
        run: crate::gamelib_abi::WorldProgressSystemFn,
        user_data: *mut c_void,
        interval_secs: f32,
    ) -> u64 {
        crate::gamelib_system::register_progress_system(
            &self.engine.world,
            name,
            query,
            run,
            user_data,
            interval_secs,
        )
    }

    #[methoddb::bind(name = "system_set_stage")]
    pub fn api_world_system_set_stage(&mut self, system_id: u64, stage: &str) -> i32 {
        crate::gamelib_system::set_system_stage(&self.engine.world, system_id, stage)
    }

    #[methoddb::bind(name = "system_set_interval")]
    pub fn api_world_system_set_interval(&mut self, system_id: u64, interval_secs: f32) -> i32 {
        crate::gamelib_system::set_progress_system_interval(
            &self.engine.world,
            system_id,
            interval_secs,
        );
        0
    }
}

#[methoddb::namespace(name = "v1:server", on_host, ctx = "server", scope = ["server"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "set_enabled")]
    pub fn api_set_server_enabled(&mut self, enabled: bool) {
        self.apply_set_server_enabled(enabled);
    }

    #[methoddb::bind(name = "activated")]
    pub fn api_server_activated(&mut self) {
        // Loopback hop → main thread connects the local client next frame.
        self.loopback
            .post_host_command(deerpunk_engine_common::network::HostCommand::InprocessConnect);
    }

    #[methoddb::bind(name = "is_enabled")]
    pub fn api_server_is_enabled(&self) -> i32 {
        if self.engine.enabled {
            1
        } else {
            0
        }
    }

    /// Monotonic simulation time (seconds) while the server is enabled.
    #[methoddb::bind(name = "sim_time_secs")]
    pub fn api_server_sim_time_secs(&self) -> f32 {
        self.engine.sim_time_secs as f32
    }
}

#[methoddb::namespace(name = "v1:physics", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    /// Rapier segment trace — returns hit entity + fraction/normal/point.
    #[methoddb::bind(name = "trace_line")]
    pub fn api_physics_trace_line(
        &mut self,
        ox: f32,
        oy: f32,
        oz: f32,
        ex: f32,
        ey: f32,
        ez: f32,
        exclude_entity: u64,
        out: &mut crate::physics_trace::PhysicsTraceResult,
    ) -> i32 {
        self.physics_trace_line(ox, oy, oz, ex, ey, ez, exclude_entity, out)
    }

    #[methoddb::bind(name = "create_shape")]
    pub fn api_physics_create_shape(&mut self, abi: &[u8]) -> u32 {
        if abi.is_empty() {
            return 0;
        }
        self.engine.physics.create_shape(abi)
    }

    #[methoddb::bind(name = "destroy_shape")]
    pub fn api_physics_destroy_shape(&mut self, handle: u32) -> i32 {
        self.engine.physics.destroy_shape(handle);
        0
    }

    #[methoddb::bind(name = "create_parts")]
    pub fn api_physics_create_parts(
        &mut self,
        parts_ptr: *const deerpunk_engine_common::physics::ColliderPart,
        count: usize,
    ) -> u32 {
        if parts_ptr.is_null() || count == 0 {
            return 0;
        }
        let parts = unsafe { std::slice::from_raw_parts(parts_ptr, count) };
        self.engine.physics.create_parts(parts)
    }

    #[methoddb::bind(name = "destroy_parts")]
    pub fn api_physics_destroy_parts(&mut self, handle: u32) -> i32 {
        self.engine.physics.destroy_parts(handle);
        0
    }

    #[methoddb::bind(name = "set_body_linvel")]
    pub fn api_physics_set_body_linvel(&mut self, entity: u64, vx: f32, vy: f32, vz: f32) -> i32 {
        self.engine.physics.set_body_linvel(entity, [vx, vy, vz]);
        0
    }

    #[methoddb::bind(name = "cast_capsule")]
    pub fn api_physics_cast_capsule(
        &mut self,
        center: *const f32,
        half_height: f32,
        radius: f32,
        dir: *const f32,
        max_dist: f32,
        exclude_entity: u64,
        out: &mut crate::engine_api::physics::CapsuleCastHitOut,
    ) -> i32 {
        if center.is_null() || dir.is_null() {
            return -1;
        }
        let center = unsafe { [*center, *center.add(1), *center.add(2)] };
        let dir = unsafe { [*dir, *dir.add(1), *dir.add(2)] };
        *out = crate::engine_api::physics::CapsuleCastHitOut::default();
        match self.engine.physics.cast_capsule(
            center,
            half_height,
            radius,
            dir,
            max_dist,
            exclude_entity,
        ) {
            Some(hit) => {
                out.hit = 1;
                out.start_solid = u32::from(hit.start_solid);
                out.toi = hit.toi;
                out.normal = hit.normal;
                out.point = hit.point;
                out.entity = hit.entity;
                1
            }
            None => 0,
        }
    }
}

#[methoddb::namespace(name = "v1:players", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "set_entity")]
    pub fn api_players_set_entity(&mut self, slot: u32, entity: u64) -> i32 {
        self.engine.players.set_entity(slot, entity);
        0
    }

    #[methoddb::bind(name = "get_usercmd")]
    pub fn api_players_get_usercmd(&self, slot: u32, out: *mut u8, out_len: usize) -> i32 {
        if out.is_null() || out_len < USERCMD_SIZE {
            return -1;
        }
        let Some(cmd) = self.engine.players.usercmd(slot) else {
            return -1;
        };
        let bytes = cmd.encode();
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, USERCMD_SIZE);
        }
        0
    }
}

#[methoddb::namespace(name = "v1:net", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "msg_register")]
    pub fn api_net_msg_register(&mut self, name: &str, size: i16) -> i32 {
        match self.um.register(name, size) {
            Ok(id) => id as i32,
            Err(e) => {
                log::warn!("[um] register: {e}");
                -1
            }
        }
    }

    #[methoddb::bind(name = "msg_send")]
    pub fn api_net_msg_send(&mut self, dest: i32, name: &str, data: &[u8]) -> i32 {
        let slot = if dest <= 0 {
            None
        } else {
            Some((dest - 1) as u32)
        };
        match self.um_send(name, data, slot) {
            Ok(()) => 0,
            Err(e) => {
                log::warn!("[um] send: {e}");
                -1
            }
        }
    }

    #[methoddb::bind(name = "pop_client_command")]
    pub fn api_net_pop_client_command(
        &mut self,
        out_slot: *mut f32,
        out_buf: *mut u8,
        out_buf_len: usize,
    ) -> i32 {
        if out_slot.is_null() || out_buf.is_null() {
            return -1;
        }
        let engine = &mut self.engine;
        let Some((slot, line)) = engine.pending_client_commands.first().cloned() else {
            return 0;
        };
        let bytes = line.as_bytes();
        if bytes.len() > out_buf_len {
            return -2;
        }
        engine.pending_client_commands.remove(0);
        unsafe {
            *out_slot = slot as f32;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
        }
        bytes.len() as i32
    }

    #[methoddb::bind(name = "player_net_id")]
    pub fn api_net_player_net_id(&self, slot: u32) -> u32 {
        let engine = &self.engine;
        let sid = engine.players.get(slot).and_then(|p| p.entity);
        match sid {
            Some(e) => engine.rep.net_id_of(e).unwrap_or(0) as u32,
            None => 0,
        }
    }
}

#[methoddb::namespace(name = "v1:cmd", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    /// Register the real handler in the facade's thread-side registry and
    /// ask the main thread for a same-name console proxy (which forwards
    /// typed lines back here via `ServerThreadCmd::ExecCmd`).
    #[methoddb::bind(name = "register")]
    pub fn api_cmd_register(
        &mut self,
        name: &str,
        callback: CmdCallback,
        user: *mut c_void,
    ) -> i32 {
        let name = name.trim();
        if name.is_empty() {
            log::error!("[cmd] register: empty name");
            return -1;
        }
        if let Some(slot) = self.console_commands.iter_mut().find(|c| c.name == name) {
            slot.callback = callback;
            slot.user = user;
        } else {
            self.console_commands.push(ConsoleCommand {
                name: name.to_string(),
                callback,
                user,
            });
        }
        log::info!("[cmd] registered server command '{name}' (sim thread)");
        let _ = self
            .server_to_host
            .send(ServerToMain::RegisterProxyCommand(name.to_string()));
        0
    }

    /// Execute a console line on the sim thread (thread-side registry).
    #[methoddb::bind(name = "execute")]
    pub fn api_cmd_execute(&mut self, command_line: &str) {
        self.exec_console_line(command_line);
    }
}

impl ServerThreadCtx {
    /// Run a console line against the facade registry (old server-thread
    /// `CmdSystem::execute_command` semantics: gamelib callback, else
    /// `name value` cvar set, else query/warn).
    pub fn exec_console_line(&mut self, line: &str) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = tokens.first() else {
            return;
        };
        let args = &tokens[1..];
        if let Some(cmd) = self
            .console_commands
            .iter()
            .find(|c| c.name == *name)
            .cloned()
        {
            let argc = args.len() as i32;
            let ptrs: Vec<*const u8> = args.iter().map(|s| s.as_ptr()).collect();
            let lens: Vec<usize> = args.iter().map(|s| s.len()).collect();
            (cmd.callback)(argc, ptrs.as_ptr(), lens.as_ptr(), cmd.user);
            return;
        }
        if !args.is_empty() {
            // GoldSrc / Xash: `name value` sets the cvar silently.
            self.cvar.set_or_create(name, &args.join(" "));
        } else if !self.cvar.print_query(name) {
            log::info!("Unknown command: {name}");
        }
    }
}

#[methoddb::namespace(name = "v1:host", on_host, ctx = "server", scope = ["server"])]
impl ServerThreadCtx {
    /// Kept for gamelib ABI compatibility: commands register on the single
    /// main CmdSystem via `v1:cmd:register`, so no proxy is needed.
    #[methoddb::bind(name = "register_main_console")]
    pub fn api_host_register_main_console(&mut self, name: &str) -> i32 {
        let _ = name;
        0
    }
}

#[methoddb::namespace(name = "v1:schedule", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "add_system")]
    pub fn api_schedule_add_system(
        &mut self,
        system_fn: crate::gamelib_abi::ScheduleSystemFn,
        user_data: *mut c_void,
    ) {
        self.gamelib_schedule
            .add_named("gamelib", CallbackSystem::new(system_fn, user_data));
    }

    #[methoddb::bind(name = "clear")]
    pub fn api_schedule_clear(&mut self) {
        self.gamelib_schedule.clear();
    }

    #[methoddb::bind(name = "run")]
    pub fn api_schedule_run(&mut self, delta_time: f32) {
        self.gamelib_schedule.run(delta_time);
    }
}

#[methoddb::namespace(name = "v1:engine", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "frame")]
    pub fn api_engine_frame(&mut self, _delta_time: f32) -> i32 {
        0
    }

    /// Gamelib `server_shutdown` hook — the thread's `shutdown` calls the DLL.
    #[methoddb::bind(name = "shutdown")]
    pub fn api_engine_shutdown(&mut self) {}
}

#[methoddb::namespace(name = "v1:profile", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "zone_begin")]
    pub fn api_profile_zone_begin(&self, name: &str) -> i32 {
        deerpunk_engine_renderer::gamelib_profile::zone_begin(name);
        0
    }

    #[methoddb::bind(name = "zone_end")]
    pub fn api_profile_zone_end(&self) -> i32 {
        deerpunk_engine_renderer::gamelib_profile::zone_end();
        0
    }

    #[methoddb::bind(name = "frame_mark")]
    pub fn api_profile_frame_mark(&self) -> i32 {
        deerpunk_engine_renderer::gamelib_profile::frame_mark();
        0
    }
}

#[methoddb::namespace(name = "v1:log", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    /// `level`: 1=error, 2=warn, 3=info, 4=debug, 5=trace.
    #[methoddb::bind(name = "write")]
    pub fn api_log_write(&self, level: i32, msg: &str) -> i32 {
        match level {
            1 => log::error!(target: "gamelib", "{msg}"),
            2 => log::warn!(target: "gamelib", "{msg}"),
            3 => log::info!(target: "gamelib", "{msg}"),
            4 => log::debug!(target: "gamelib", "{msg}"),
            5 => log::trace!(target: "gamelib", "{msg}"),
            _ => log::info!(target: "gamelib", "{msg}"),
        }
        0
    }
}

#[methoddb::namespace(name = "v1:cvar", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "set")]
    pub fn api_cvar_set(&self, name: &str, value: &str) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.cvar.set_or_create(name, value);
        0
    }

    #[methoddb::bind(name = "get_f32")]
    pub fn api_cvar_get_f32(&self, name: &str, out: &mut f32) -> i32 {
        if name.is_empty() {
            return -1;
        }
        match self.cvar.get_f32(name) {
            Some(v) => {
                *out = v;
                0
            }
            None => -1,
        }
    }
}

#[methoddb::namespace(name = "v1:audio", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "play_2d")]
    pub fn api_audio_play_2d(&mut self, event: &str, volume_u8: u32) -> u32 {
        let id = self.alloc_sound_id();
        self.queue_sound_packet(
            SoundPlayRpc::play_2d(id, event, volume_u8 as f32 / 255.0, Vec::new()).to_packet(),
        );
        id
    }

    #[methoddb::bind(name = "play_3d")]
    pub fn api_audio_play_3d(
        &mut self,
        event: &str,
        x: f32,
        y: f32,
        z: f32,
        volume_u8: u32,
        param_name: &str,
        param_value: f32,
    ) -> u32 {
        let params = if param_name.is_empty() {
            Vec::new()
        } else {
            vec![(param_name.to_string(), param_value)]
        };
        let id = self.alloc_sound_id();
        let rpc = SoundPlayRpc::play_3d(id, event, [x, y, z], volume_u8 as f32 / 255.0, params);
        self.queue_sound_packet(rpc.to_packet());
        id
    }

    #[methoddb::bind(name = "start_attached")]
    pub fn api_audio_start_attached(
        &mut self,
        entity: u64,
        event: &str,
        x: f32,
        y: f32,
        z: f32,
        volume_u8: u32,
    ) -> u32 {
        let id = self.alloc_sound_id();
        let net_id = self.engine.rep.net_id_of(entity).unwrap_or(0);
        let rpc = SoundPlayRpc::play_loop(id, event, [x, y, z], volume_u8 as f32 / 255.0, net_id);
        self.queue_sound_packet(rpc.to_packet());
        id
    }

    #[methoddb::bind(name = "stop_sound")]
    pub fn api_audio_stop_sound(&mut self, sound_id: u32) -> i32 {
        if sound_id == 0 {
            return -1;
        }
        self.queue_sound_packet(SoundStopRpc::new(sound_id).to_packet());
        0
    }
}

#[methoddb::namespace(name = "v1:particles", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    /// Emit a particle burst from the server to all play-phase clients.
    #[methoddb::bind(name = "emit")]
    pub fn api_particles_emit(
        &mut self,
        effect: &str,
        x: f32,
        y: f32,
        z: f32,
        vx: f32,
        vy: f32,
        vz: f32,
        payload_json: &str,
    ) -> u32 {
        let payload: deerpunk_engine_particles::SpawnPayload = if payload_json.trim().is_empty() {
            Default::default()
        } else {
            match serde_json::from_str(payload_json) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("[particles] emit '{effect}': bad payload json: {e}");
                    return 0;
                }
            }
        };
        self.particles_emit(effect, [x, y, z], [vx, vy, vz], &payload);
        1
    }
}

/// `v1:client_world` on the sim thread. The old server-thread Host served
/// these with the client engine absent: `component_register` registered
/// "Both" components into the **server** world; every other op was a neutral
/// no-op (no client ECS on this thread). Facade behavior matches that.
#[methoddb::namespace(name = "v1:client_world", on_host, ctx = "server", scope = ["server", "client"])]
impl ServerThreadCtx {
    #[methoddb::bind(name = "get")]
    pub fn api_client_world_get(&self, _entity: u64, _comp: u64) -> *mut c_void {
        std::ptr::null_mut()
    }

    #[methoddb::bind(name = "set")]
    pub fn api_client_world_set(
        &mut self,
        _entity: u64,
        _comp: u64,
        _data: *const c_void,
        _size: usize,
    ) {
    }

    #[methoddb::bind(name = "has")]
    pub fn api_client_world_has(&self, _entity: u64, _comp: u64) -> bool {
        false
    }

    #[methoddb::bind(name = "is_alive")]
    pub fn api_client_world_is_alive(&self, _entity: u64) -> bool {
        false
    }

    #[methoddb::bind(name = "spawn")]
    pub fn api_client_world_spawn(&self) -> u64 {
        0
    }

    #[methoddb::bind(name = "destroy")]
    pub fn api_client_world_destroy(&mut self, _entity: u64) {}

    #[methoddb::bind(name = "remove")]
    pub fn api_client_world_remove(&mut self, _entity: u64, _comp: u64) {}

    /// "Both" component registration lands on the server world here.
    #[methoddb::bind(name = "component_register")]
    pub fn api_client_world_component_register(
        &self,
        desc: &deerpunk_engine_ffi::ComponentRegisterDesc,
    ) -> u64 {
        let Some(reg) = crate::engine_api::components::build_component_descriptor(desc) else {
            return 0;
        };
        self.engine.world.component_init_raw(&reg)
    }

    #[methoddb::bind(name = "query")]
    pub fn api_client_world_query(
        &mut self,
        _require: &[u64],
        _exclude: &[u64],
        _out: *mut u64,
        _max: usize,
    ) -> usize {
        0
    }

    #[methoddb::bind(name = "system_register")]
    pub fn api_client_world_system_register(
        &mut self,
        _name: &str,
        _query: &str,
        _run: crate::gamelib_abi::WorldProgressSystemFn,
        _user_data: *mut c_void,
        _interval_secs: f32,
    ) -> u64 {
        0
    }

    #[methoddb::bind(name = "system_set_stage")]
    pub fn api_client_world_system_set_stage(&mut self, _system_id: u64, _stage: &str) -> i32 {
        0
    }

    #[methoddb::bind(name = "system_set_interval")]
    pub fn api_client_world_system_set_interval(
        &mut self,
        _system_id: u64,
        _interval_secs: f32,
    ) -> i32 {
        0
    }

    #[methoddb::bind(name = "register_observer")]
    pub fn api_client_world_register_observer(
        &mut self,
        _query_ptr: *mut u8,
        _query_len: usize,
        _events: u32,
        _callback_ptr: u64,
    ) -> i32 {
        0
    }
}
