deerpunk_engine_common::cvar_def!(
    "sv_cheats",
    "0",
    "Allow cheat commands (noclip). 0=off, 1=on."
);

deerpunk_engine_common::cvar_def!(
    "ph_capsule_skin",
    "0.5",
    "Contact skin (target_distance) for swept capsule casts. \
     Smaller = tighter collision (voxel 1-unit scale), larger = air cushion (HL scale)."
);

use std::collections::HashSet;

use deerpunk_engine_common::network::SlotPlayGate;
use deerpunk_engine_common::physics::{NullPhysics, PhysicsBackend, RapierPhysicsBackend};
use deerpunk_engine_common::world::World;

use crate::lan_advertiser::LanAdvertiser;
use crate::network::ServerPrefabReplication;
use crate::players::PlayerConnections;
use crate::server_net::ServerNet;
use crate::ServerDll;

mod profile {
    pub struct Scope;
    impl Scope {
        #[inline]
        pub fn new(_name: &'static str) -> Self {
            Self
        }
    }
}

/// Server simulation + session (players, gamelib DLL, replication, LAN host).
pub struct ServerEngine {
    pub world: World,
    pub physics: Box<dyn PhysicsBackend>,
    pub(crate) initialized: bool,
    pub enabled: bool,
    terrain_bodies: HashSet<u64>,
    pub players: PlayerConnections,
    pub dll: Option<ServerDll>,
    pub rep: ServerPrefabReplication,
    pub lan_advertiser: Option<LanAdvertiser>,
    pub lan_map: String,
    /// Monotonic ids for `SoundPlay` / `SoundStop` (server authority).
    pub next_sound_id: u32,
    /// Slots allowed to send `UserCmd` after join init.
    pub slots_play: SlotPlayGate,
    pub network: ServerNet,
    /// Monotonic server sim tick (for UserCmdAck).
    pub sim_tick: u32,
    /// Queued `CLC::ClientCommand` lines per player slot (drained by server gamelib).
    pub pending_client_commands: Vec<(u32, String)>,
}

impl ServerEngine {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            physics: Box::new(NullPhysics),
            initialized: false,
            enabled: false,
            terrain_bodies: HashSet::new(),
            players: PlayerConnections::default(),
            dll: None,
            rep: ServerPrefabReplication::new(),
            lan_advertiser: None,
            lan_map: String::new(),
            next_sound_id: 1,
            slots_play: SlotPlayGate::default(),
            network: ServerNet::default(),
            sim_tick: 0,
            pending_client_commands: Vec::new(),
        }
    }

    pub fn initialize(&mut self, _gamedir: impl AsRef<std::path::Path>) -> Result<(), String> {
        if self.initialized {
            return Err("Engine already initialized".to_string());
        }
        let mut backend = Box::new(RapierPhysicsBackend::new());
        if !backend.init() {
            return Err("physics backend init failed".into());
        }
        backend.attach_ecs(&self.world);
        self.physics = backend;
        self.initialized = true;
        Ok(())
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        log::info!("[server] world enabled={on}");
    }

    /// Prefab replication tick → SVC payloads for all connected clients (listen / dedicated).
    pub fn replication_packets(&mut self) -> Vec<Vec<u8>> {
        if !self.enabled {
            return Vec::new();
        }
        self.rep.tick(&self.world)
    }

    pub fn on_udp_player_connected(&mut self, slot: u32) {
        self.players
            .connect_remote(slot, format!("player{slot}"));
    }

    pub fn on_udp_player_disconnected(&mut self, slot: u32) {
        self.slots_play.remove(slot);
        self.players.disconnect_slot(slot);
    }

    pub fn frame(&mut self, delta_time: f32) -> Result<(), String> {
        if !self.initialized {
            return Err("Engine not initialized".to_string());
        }
        if !self.enabled {
            return Ok(());
        }
        {
            let _g = profile::Scope::new("server.ecs");
            self.world.simulate(delta_time);
        }
        {
            let _g = profile::Scope::new("physics.sim.step");
            self.physics.step(delta_time);
        }
        {
            let _g = profile::Scope::new("physics.prepare");
            self.physics.prepare(&self.world, delta_time);
        }
        {
            let _g = profile::Scope::new("physics.writeback");
            self.physics.writeback(&self.world);
        }
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if self.initialized {
            self.physics.shutdown();
            self.initialized = false;
            self.enabled = false;
        }
    }
}

impl Default for ServerEngine {
    fn default() -> Self {
        Self::new()
    }
}
