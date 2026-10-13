//! Scene-level lighting — not per-material.
//!
//! `sun_dir` / `ambient` were previously baked into `Material`, which meant
//! changing scene lighting touched every material copy. Now one
//! `SceneLighting` per frame, uploaded as a global uniform. Skybox / unlit
//! shaders ignore it.

/// Scene-level directional + ambient lighting. One per frame.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SceneLighting {
    /// xyz = sun direction (normalized), w = intensity (0 = sun off).
    pub sun_dir: [f32; 4],
    /// Ambient light color (linear RGB, alpha unused).
    pub ambient: [f32; 4],
}

impl Default for SceneLighting {
    fn default() -> Self {
        Self {
            // No sun by default; gamelib sets from light_environment / worldspawn.
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0, 0.0, 0.0, 0.0],
        }
    }
}
