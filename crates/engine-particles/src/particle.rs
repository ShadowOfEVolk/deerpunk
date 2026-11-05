use glam::Vec3;

/// One simulated billboard particle (world-space position).
#[derive(Clone, Debug)]
pub struct LiveParticle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub lifetime: f32,
    /// Game effect catalog index (opaque to the engine).
    pub effect: u8,
    pub tile: u8,
    pub half: f32,
    pub rot: f32,
    pub spin: f32,
    pub phase: f32,
    pub phase_rate: f32,
    pub tint: [f32; 3],
    pub opacity: f32,
    pub uv_min: [f32; 2],
    pub uv_span: f32,
}
