use std::collections::{HashMap, HashSet};

use deerpunk_engine_common::physics::PhysicsWorld;

use crate::world::World;

pub struct ServerEngine {
    pub world: World,
    pub physics_world: PhysicsWorld,
    pub(crate) initialized: bool,
    terrain_bodies: HashSet<u64>,
}

impl ServerEngine {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            physics_world: PhysicsWorld::new(),
            initialized: false,
            terrain_bodies: HashSet::new(),
        }
    }

    pub fn initialize(&mut self, gamedir: impl AsRef<std::path::Path>) -> Result<(), String> {
        if self.initialized {
            return Err("Engine already initialized".to_string());
        }
        self.initialized = true;
        Ok(())
    }

    pub fn frame(&self, delta_time: f32) -> Result<(), String> {
        if !self.initialized {
            return Err("Engine not initialized".to_string());
        }
        self.world.simulate(delta_time);
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if self.initialized {
            self.initialized = false;
        }
    }
}

impl Default for ServerEngine {
    fn default() -> Self {
        Self::new()
    }
}
