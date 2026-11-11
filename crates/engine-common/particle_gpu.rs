//! GPU contract for the native instanced particle pass.
//!
//! The client sim owns a dense particle pool (no ECS entities, no meshes).
//! Once per frame it packs live particles into one instance buffer payload
//! ([`ParticleFrame`]) that the renderer draws with a single static quad and
//! a billboard vertex shader: the camera-facing basis is built on the GPU, so
//! the same buffer serves every camera.

use std::sync::Arc;

/// Per-particle instance payload, 40 bytes, no implicit padding.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    /// World-space quad center.
    pub center: [f32; 3],
    /// Quad half-size (the quad is square; `half` scales the static quad).
    pub half: f32,
    /// In-plane spin, radians. Applied in the vertex shader.
    pub rot: f32,
    /// Sprite rect `[u0, v0, u1, v1]` in the bound texture.
    pub uv_rect: [f32; 4],
    /// `unorm8x4` tint; alpha carries fade × opacity for the dither cut.
    pub tint: [u8; 4],
}

pub const GPU_PARTICLE_STRIDE: u32 = 40;

/// One instanced draw over the frame's instance buffer: `len` instances
/// starting at `offset`, bound to `tex_uri`. Ranges keep pool order grouped
/// by texture, so one buffer write per frame serves all draws.
#[derive(Clone)]
pub struct ParticleDrawRange {
    pub tex_uri: Arc<str>,
    pub offset: u32,
    pub len: u32,
}

/// Per-frame particle payload (client sim → renderer).
#[derive(Default, Clone)]
pub struct ParticleFrame {
    pub instances: Vec<GpuParticle>,
    pub ranges: Vec<ParticleDrawRange>,
}

impl ParticleFrame {
    pub fn clear(&mut self) {
        self.instances.clear();
        self.ranges.clear();
    }
}
