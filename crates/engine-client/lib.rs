// Client-side engine components
mod cvars;
pub mod client_dll;
pub mod demo;
pub mod lan_browser;
pub mod entity_debug;
pub mod phys_debug;
pub mod render_gpu_log;
pub mod render_sync;
pub mod network;
pub mod systems;

pub mod camera;
pub mod audio;

pub mod common {
    pub use deerpunk_engine_common::*;
}

// pub use network::netchan::{IncomingMessage, NetChan};
// pub use network::replication::{client_replication, ClientReplication};
pub use camera::{Camera, CameraMode, FreecamController, CAMERA_SMOOTH_SPEED, FOLLOW_VIEW_SIZE, tank_in_camera_view};
pub mod app;
pub mod client_engine;
pub mod console;
pub mod local_view;
pub mod local_sim;
pub mod usercmd_channel;

pub use client_dll::{ClientDll, ClientInitData};
pub use deerpunk_engine_server::ClientNet;
pub mod draw_material_resolve;
pub mod imgui_console_input;
pub mod infobox;
pub mod mesh_alloc_debug;
pub mod bone_buffer_pool;
pub mod mesh_lifecycle;
pub mod render_extract_cache;
pub mod render_extractor;
pub mod scene;
pub mod ui_controller;

pub use ui_controller::UiController;
pub use audio::{AudioBackend, AudioInitContext, AudioListener, FmodEventInfo, NullAudio};

pub use deerpunk_engine_renderer as renderer;

extern crate log;
