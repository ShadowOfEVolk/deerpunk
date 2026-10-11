//! Studio sound emitter attached to an entity (position from TransformComponent).

/// Fixed-size event path: NUL-padded UTF-8, max 127 chars + NUL.
pub const SOUND_EVENT_MAX: usize = 128;

/// FMOD Studio event source on an entity. Replicated server → client.
/// Playback state (started/stopped) is client-side runtime, not stored here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SoundEmitter {
    /// Studio event path, e.g. `event:/ambient/fan` (NUL-padded).
    pub event: [u8; SOUND_EVENT_MAX],
    /// Event volume multiplier (0..=1).
    pub volume: f32,
    /// Start automatically when the entity appears on the client.
    pub autoplay: bool,
    pub _pad: [u8; 3],
}

impl SoundEmitter {
    pub fn new(event: &str, volume: f32, autoplay: bool) -> Self {
        let mut buf = [0u8; SOUND_EVENT_MAX];
        let bytes = event.as_bytes();
        let n = bytes.len().min(SOUND_EVENT_MAX - 1);
        buf[..n].copy_from_slice(&bytes[..n]);
        Self {
            event: buf,
            volume: volume.clamp(0.0, 1.0),
            autoplay,
            _pad: [0; 3],
        }
    }

    /// Event path as `&str` (up to the first NUL).
    pub fn event_str(&self) -> &str {
        let end = self.event.iter().position(|&b| b == 0).unwrap_or(0);
        std::str::from_utf8(&self.event[..end]).unwrap_or("")
    }
}
