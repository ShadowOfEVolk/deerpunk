//! Audio abstraction for the client.
//!
//! The [`AudioBackend`] trait lives here; concrete implementations (FMOD, …)
//! live in separate crates and are plugged in via
//! [`crate::client_engine::ClientEngine::set_audio_backend`].

use std::path::Path;

/// View-space listener for 3D audio (world units = HL units).
#[derive(Clone, Copy, Debug)]
pub struct AudioListener {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

impl Default for AudioListener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            forward: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }
}

/// Entity-attached looping sound: `(backend_key, origin)` pairs refreshed per frame.
pub type AttachedSounds<'a> = &'a [(u64, [f32; 3])];

/// One Studio event enumerated from loaded banks (inspector / tools).
#[derive(Clone, Debug, Default)]
pub struct FmodEventInfo {
    pub path: String,
    pub bank: String,
    pub is_3d: bool,
    pub oneshot: bool,
    pub params: Vec<String>,
}

/// Pluggable audio backend. Single-threaded (client tick thread).
pub trait AudioBackend {
    /// Load banks / start runtime. `gamedir` = game content directory
    /// (banks expected in `{gamedir}/sound`). `false` → backend disabled.
    fn init(&mut self, gamedir: &Path) -> bool;

    /// Per-frame update: listener, attached sound positions, master volume (0..1).
    fn update(&mut self, listener: &AudioListener, attached: AttachedSounds, volume: f32);

    /// One-shot Studio event at the listener (UI / non-positional).
    fn play_event_2d(&mut self, event: &str, volume: f32, params: &[(&str, f32)]);

    /// One-shot Studio event at a world position.
    fn play_event_3d(
        &mut self,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    );

    /// Start a looping Studio event attached to `key` (entity id). Replaces
    /// a previous event on the same key.
    fn start_attached(&mut self, key: u64, event: &str, origin: [f32; 3], volume: f32);

    /// Stop (fade out) and free the attached event for `key`, if any.
    fn stop_attached(&mut self, key: u64);

    /// Install the world geometry scene for simulation (Steam Audio etc.).
    /// Called when the client world receives a BSP map mesh.
    fn set_world_scene(
        &mut self,
        _mesh: &deerpunk_engine_common::resource::mesh::Mesh,
        _debug: &deerpunk_engine_common::bsp_debug::BspWorldDebug,
    ) {
    }

    /// Stop everything and release the runtime.
    fn shutdown(&mut self);

    /// All Studio events from loaded banks (empty if runtime not ready).
    fn list_events(&self) -> Vec<FmodEventInfo> {
        Vec::new()
    }
}

/// No-op backend — used until a real one is installed.
#[derive(Default)]
pub struct NullAudio;

impl AudioBackend for NullAudio {
    fn init(&mut self, _gamedir: &Path) -> bool {
        false
    }

    fn update(&mut self, _listener: &AudioListener, _attached: AttachedSounds, _volume: f32) {}

    fn play_event_2d(&mut self, _event: &str, _volume: f32, _params: &[(&str, f32)]) {}

    fn play_event_3d(
        &mut self,
        _event: &str,
        _origin: [f32; 3],
        _volume: f32,
        _params: &[(&str, f32)],
    ) {
    }

    fn start_attached(&mut self, _key: u64, _event: &str, _origin: [f32; 3], _volume: f32) {}

    fn stop_attached(&mut self, _key: u64) {}

    fn shutdown(&mut self) {}
}
