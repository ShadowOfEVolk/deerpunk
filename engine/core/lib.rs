// Core engine framework
// Can include dedicated server or server with client

use std::sync::Mutex;
pub use legion::*;

pub mod common {
    pub use drifter_engine_common::*;
}

#[cfg(feature = "dedicated-server")]
pub mod server {
    pub use drifter_engine_server::*;
}

#[cfg(feature = "client")]
pub mod client {
    pub use drifter_engine_client::*;
}

// Singleton systems from common
use drifter_engine_common::{CmdSystem, CvarSystem, FileSystem, cmd::cmd, cvar::cvar, fs::fs};

pub struct Engine {
    initialized: bool,
    world: World,
    schedule: Schedule,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            initialized: false,
            world: World::default(),
            schedule: Schedule::builder().build(),
        }
    }

    /// Get reference to ECS World
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Get mutable reference to ECS World
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Execute ECS schedule
    pub fn run_schedule(&mut self) {
        let mut resources = Resources::default();
        self.schedule.execute(&mut self.world, &mut resources);
    }

    pub fn initialize<P: AsRef<std::path::Path>>(&mut self, gamedir: P) -> Result<(), String> {
        if self.initialized {
            return Err("Engine already initialized".to_string());
        }

        fs().initialize();
        fs().set_gamedir(gamedir);

        cmd().initialize();
        cvar().initialize();
        
        println!("Engine initialized successfully");
        
        self.initialized = true;
        Ok(())
    }

    pub fn frame(&mut self, _delta_time: f32) -> Result<(), String> {
        if !self.initialized {
            return Err("Engine not initialized".to_string());
        }
        
        // Process command system frame
        cmd().frame();
        
        // Execute ECS schedule
        // Systems can access delta_time via singleton or pass as parameter
        self.run_schedule();
        
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if self.initialized {
            // Shutdown singleton systems
            cmd().shutdown();
            cvar().shutdown();
            fs().shutdown();
            
            println!("Engine shutdown");
            self.initialized = false;
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

