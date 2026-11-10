//! Rapier trace helpers (`v1:physics:trace_line`).
//!
//! The trace itself lives on `ServerThreadCtx` (the physics backend is on the
//! sim thread); this file only holds the FFI out-param layout.

/// Out-param for `v1:physics:trace_line` (gamelib mirrors this layout).
#[repr(C)]
pub struct PhysicsTraceResult {
    pub hit: u32,
    pub entity: u64,
    pub fraction: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}
