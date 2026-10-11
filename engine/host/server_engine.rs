use std::collections::HashSet;

use deerpunk_engine_common::physics::{NullPhysics, PhysicsBackend, RapierPhysicsBackend};

use crate::world::World;

pub struct ServerEngine {
    pub world: World,
    pub physics: Box<dyn PhysicsBackend>,
    pub(crate) initialized: bool,
    /// When false, host does not simulate / call server_frame game logic.
    pub enabled: bool,
    terrain_bodies: HashSet<u64>,
}

impl ServerEngine {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            physics: Box::new(NullPhysics),
            initialized: false,
            enabled: false,
            terrain_bodies: HashSet::new(),
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

    pub fn frame(&mut self, delta_time: f32) -> Result<(), String> {
        if !self.initialized {
            return Err("Engine not initialized".to_string());
        }
        if !self.enabled {
            return Ok(());
        }
        // Flecs systems (if any).
        self.world.simulate(delta_time);

        // step first so newly OnSet colliders enter the BVH before KCC queries.
        // (Rapier query pipeline is stale until broadphase update inside step.)
        {
            let _g = crate::system_profiler::scope("physics.sim.step");
            self.physics.step(delta_time);
        }
        {
            let _g = crate::system_profiler::scope("physics.prepare");
            self.physics.prepare(&self.world, delta_time);
        }
        {
            let _g = crate::system_profiler::scope("physics.writeback");
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
