//! Audio abstraction for the client.
//!
//! The [`AudioBackend`] trait lives here; concrete implementations (FMOD, …)
//! live in separate crates and are plugged in via
//! [`crate::client_engine::ClientEngine::set_audio_backend`].

use std::path::Path;

use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::sound_event::SoundEventRegistry;

/// Passed to [`AudioBackend::init`].
pub struct AudioInitContext<'a> {
    pub gamedir: &'a Path,
}

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
    /// Load banks / start runtime. `false` → backend disabled.
    fn init(&mut self, ctx: &AudioInitContext) -> bool;

    /// Merge datapack / gamelib sound events (Kira; no-op for FMOD).
    fn merge_sound_events(&mut self, _registry: &SoundEventRegistry) {}

    /// Per-frame update: listener, attached sound positions, master volume (0..1).
    fn update(&mut self, listener: &AudioListener, attached: AttachedSounds, volume: f32);

    /// One-shot Studio event at the listener (UI / non-positional).
    fn play_event_2d(
        &mut self,
        assets: &ResourceManager,
        event: &str,
        volume: f32,
        params: &[(&str, f32)],
    );

    /// One-shot Studio event at a world position.
    fn play_event_3d(
        &mut self,
        assets: &ResourceManager,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    );

    /// Start a looping Studio event attached to `key` (entity id). Replaces
    /// a previous event on the same key.
    fn start_attached(
        &mut self,
        assets: &ResourceManager,
        key: u64,
        event: &str,
        origin: [f32; 3],
        volume: f32,
    );

    /// Stop (fade out) and free the attached event for `key`, if any.
    fn stop_attached(&mut self, key: u64);

    /// Install world geometry for simulation (Steam Audio etc.) — mesh only.
    fn set_world_scene(&mut self, _mesh: &deerpunk_engine_common::resource::mesh::Mesh) {}

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
    fn init(&mut self, _ctx: &AudioInitContext) -> bool {
        false
    }

    fn update(&mut self, _listener: &AudioListener, _attached: AttachedSounds, _volume: f32) {}

    fn play_event_2d(
        &mut self,
        _assets: &ResourceManager,
        _event: &str,
        _volume: f32,
        _params: &[(&str, f32)],
    ) {
    }

    fn play_event_3d(
        &mut self,
        _assets: &ResourceManager,
        _event: &str,
        _origin: [f32; 3],
        _volume: f32,
        _params: &[(&str, f32)],
    ) {
    }

    fn start_attached(
        &mut self,
        _assets: &ResourceManager,
        _key: u64,
        _event: &str,
        _origin: [f32; 3],
        _volume: f32,
    ) {
    }

    fn stop_attached(&mut self, _key: u64) {}

    fn shutdown(&mut self) {}
}
