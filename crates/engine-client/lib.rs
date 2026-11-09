// Client-side engine components
pub mod client_dll;
mod cvars;
pub mod demo;
pub mod entity_debug;
pub mod lan_browser;
pub mod network;
pub mod phys_debug;
pub mod render_gpu_log;
pub mod render_sync;
pub mod systems;

pub mod audio;
pub mod camera;

pub mod common {
    pub use deerpunk_engine_common::*;
}

pub use camera::{
    tank_in_camera_view, Camera, CameraMode, FreecamController, CAMERA_SMOOTH_SPEED,
    FOLLOW_VIEW_SIZE,
};
pub mod app;
pub mod camera_compose;
pub mod camera_util;
pub mod client_engine;
pub mod console;
pub mod local_sim;
pub mod local_view;
pub mod usercmd_channel;

pub use client_dll::{ClientDll, ClientInitData};
pub use deerpunk_engine_server::ClientNet;
pub mod bone_buffer_pool;
pub mod draw_material_resolve;
pub mod gamelib_ui;
pub mod infobox;
pub mod mesh_alloc_debug;
pub mod mesh_lifecycle;
pub mod render_extractor;
pub mod render_thread;
pub mod scene;
pub mod ui_controller;
mod ui_rml_input;

pub use audio::{AudioBackend, AudioInitContext, AudioListener, FmodEventInfo, NullAudio};
pub use ui_controller::UiController;

pub use deerpunk_engine_renderer as renderer;

extern crate log;
