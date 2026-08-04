// Core engine framework
// Can include dedicated server or server with client

use glam::Vec3;
pub use hecs::*;

pub mod schedule;
pub use schedule::Schedule;

pub mod common {
    pub use deerpunk_engine_common::*;
}

#[cfg(feature = "dedicated-server")]
pub mod server {
    pub use deerpunk_engine_server::*;
}

#[cfg(feature = "client")]
pub mod client {
    pub use deerpunk_engine_client::*;
}

// Singleton systems from common
use deerpunk_engine_common::{cmd::cmd, cvar::cvar, fs::fs, CmdSystem, CvarSystem, FileSystem};

#[cfg(feature = "client")]
use deerpunk_engine_common::{SimpleEntity, TransformComponent};

pub struct Engine {
    initialized: bool,
    server_world: World,
    schedule: Schedule,
    // #[cfg(feature = "client")]
    // client_world: World,
    // #[cfg(feature = "client")]
    // server_frames: deerpunk_engine_server::network::ServerFrames,
    // #[cfg(feature = "client")]
    // client_frames: deerpunk_engine_client::network::frames::ClientFrames,
    // #[cfg(feature = "client")]
    // server_netchan: deerpunk_engine_server::network::NetChan,
    // #[cfg(feature = "client")]
    // client_netchan: deerpunk_engine_client::network::NetChan,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            initialized: false,
            server_world: World::new(),
            schedule: {
                #[cfg(feature = "client")]
                {
                    use deerpunk_engine_client::simple_entity_system;
                    use deerpunk_engine_server::simple_circle_renderer_system;
                    let mut schedule = Schedule::new();
                    schedule
                        .add_system(simple_entity_system)
                        .add_system(simple_circle_renderer_system);
                    schedule
                }
                #[cfg(not(feature = "client"))]
                {
                    Schedule::new()
                }
            },
            // #[cfg(feature = "client")]
            // client_world: World::new(),
            // #[cfg(feature = "client")]
            // server_frames: deerpunk_engine_server::network::ServerFrames::new(),
            // #[cfg(feature = "client")]
            // client_frames: deerpunk_engine_client::network::frames::ClientFrames::new(),
            // #[cfg(feature = "client")]
            // server_netchan: deerpunk_engine_server::network::NetChan::new(0),
            // #[cfg(feature = "client")]
            // client_netchan: deerpunk_engine_client::network::NetChan::new(0),
        }
    }

    /// Get reference to server ECS World
    pub fn server_world(&self) -> &World {
        &self.server_world
    }

    /// Get mutable reference to server ECS World
    pub fn server_world_mut(&mut self) -> &mut World {
        &mut self.server_world
    }

    /// Register a system into the engine schedule (runs on the server world each frame).
    pub fn add_system<S: crate::schedule::System + 'static>(&mut self, system: S) -> &mut Self {
        self.schedule.add_system(system);
        self
    }

    /// Remove all systems from the schedule (drops built-in demo systems if desired).
    pub fn clear_systems(&mut self) -> &mut Self {
        self.schedule.clear();
        self
    }

    /// Get reference to client ECS World
    // #[cfg(feature = "client")]
    // pub fn client_world(&self) -> &World {
    //     &self.client_world
    // }

    /// Get mutable reference to client ECS World
    // #[cfg(feature = "client")]
    // pub fn client_world_mut(&mut self) -> &mut World {
    //     &mut self.client_world
    // }

    /// Execute ECS systems (runs on server world)
    pub fn run_schedule(&mut self, delta_time: f32) {
        self.schedule.run(&mut self.server_world, delta_time);
    }

    /// Process network communication (loopback with latency and jitter)
    // #[cfg(feature = "client")]
    // fn process_network(&mut self) {
    //     use deerpunk_engine_common::network::protocol::{CLC, SVC};

    //     // Process pending messages (check if any are ready to be delivered)
    //     self.server_netchan.process_pending();
    //     self.client_netchan.process_pending();

    //     // Process incoming messages from client
    //     for msg in self.server_netchan.process_incoming() {
    //         match msg.cmd {
    //             cmd if cmd == CLC::CurrentFrame as u8 => {
    //                 // Client sent current frame
    //                 if msg.payload.len() >= 1 {
    //                     let known_frame = msg.payload[0];
    //                     self.server_frames
    //                         .set_client_known_frame(self.server_netchan.client_id, known_frame);
    //                 }
    //             }
    //             _ => {
    //                 eprintln!("Unknown client command: {}", msg.cmd);
    //             }
    //         }
    //     }

    //     // Send delta packet to client
    //     let known_frame = self
    //         .server_frames
    //         .get_client_known_frame(self.server_netchan.client_id);
    //     let current_frame = self.server_frames.current_frame();

    //     let mut buffer = Vec::new();
    //     if let Err(e) = self
    //         .server_frames
    //         .write_delta_packet(&mut buffer, self.server_netchan.client_id)
    //     {
    //         eprintln!("Failed to write delta packet: {}", e);
    //         return;
    //     }

    //     if !buffer.is_empty() {
    //         let packet_size = buffer.len();
    //         println!(
    //             "Delta packet: frame_from={}, frame_to={}, size={} bytes",
    //             known_frame, current_frame, packet_size
    //         );
    //         self.server_netchan
    //             .send_message(SVC::DeltaPacketEntities, buffer);
    //     }

    //     // Loopback: transfer messages between server and client (with latency/jitter)
    //     while let Some(msg) = self.server_netchan.get_outgoing_message() {
    //         self.client_netchan.receive_message(msg);
    //     }

    //     while let Some(msg) = self.client_netchan.get_outgoing_message() {
    //         self.server_netchan.receive_message(msg);
    //     }
    // }

    /// Sync server world state to client world using frame-based delta replication
    // #[cfg(feature = "client")]
    // fn sync_server_to_client(&mut self) {
    //     use deerpunk_engine_client::client_replication;
    //     use deerpunk_engine_common::network::protocol::{CLC, SVC};
    //     use std::io::Cursor;
    //     use std::io::Read;

    //     // Process incoming messages from server
    //     for msg in self.client_netchan.process_incoming() {
    //         match msg.cmd {
    //             cmd if cmd == SVC::DeltaPacketEntities as u8 => {
    //                 // Apply delta packet to client world
    //                 let mut cursor = Cursor::new(&msg.payload);

    //                 match client_replication()
    //                     .parse_delta_entities_packet(&mut cursor, &mut self.client_world)
    //                 {
    //                     Ok((_frame_from, frame_to)) => {
    //                         // Create client frame after applying delta
    //                         self.client_frames.create_frame(&mut self.client_world);

    //                         // Update client's known frame - send back to server
    //                         self.client_netchan
    //                             .send_message(CLC::CurrentFrame, vec![frame_to]);
    //                     }
    //                     Err(e) => {
    //                         eprintln!("Failed to parse delta packet: {}", e);
    //                     }
    //                 }
    //             }
    //             _ => {
    //                 eprintln!("Unknown server command: {}", msg.cmd);
    //             }
    //         }
    //     }
    // }

    pub fn initialize<P: AsRef<std::path::Path>>(&mut self, gamedir: P) -> Result<(), String> {
        if self.initialized {
            return Err("Engine already initialized".to_string());
        }

        fs().initialize();
        fs().set_gamedir(gamedir);

        cmd().initialize();
        cvar().initialize();

        // Конфиг игрока: configs/autoexec.cfg в CWD (если есть).
        cmd().exec_config_file("configs/autoexec.cfg");

        let center_position = Vec3::new(400.0, 300.0, 0.0);

        // Spawn SimpleEntity on server
        #[cfg(feature = "client")]
        {
            use deerpunk_engine_common::SimpleCircleRenderer;
            let server_entity = self.server_world.spawn((
                TransformComponent {
                    position: center_position,
                    ..Default::default()
                }, // Center of screen
                SimpleEntity::new(),
                SimpleCircleRenderer::default(),
            ));
            println!("Spawned server SimpleEntity: {:?}", server_entity);
        }

        println!("Engine initialized successfully");

        self.initialized = true;
        Ok(())
    }

    pub fn frame(&mut self, delta_time: f32) -> Result<(), String> {
        if !self.initialized {
            return Err("Engine not initialized".to_string());
        }

        // Process command system frame
        cmd().frame();

        // Execute ECS schedule with delta_time
        self.run_schedule(delta_time);

        // Process network communication (loopback)
        // #[cfg(feature = "client")]
        // {
        //     self.process_network();
        //     self.sync_server_to_client();
        // }

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
