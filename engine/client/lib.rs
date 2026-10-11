// Client-side engine components
// pub mod network;
pub mod systems;

pub mod camera;
pub mod audio;
pub mod sky;

pub mod common {
    pub use deerpunk_engine_common::*;
}

// pub use network::netchan::{IncomingMessage, NetChan};
// pub use network::replication::{client_replication, ClientReplication};
pub use camera::{Camera, CameraMode, FreecamController, CAMERA_SMOOTH_SPEED, FOLLOW_VIEW_SIZE, tank_in_camera_view};
pub mod app;
pub mod client_engine;
pub mod render_extractor;
pub mod ui_controller;

pub use ui_controller::UiController;
pub use audio::{AudioBackend, AudioListener, FmodEventInfo, NullAudio};

pub use deerpunk_engine_renderer as renderer;

extern crate log;
