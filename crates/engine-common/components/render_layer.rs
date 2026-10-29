//! Bitmask: which camera passes may draw this entity (`draw.mask & camera.layer_mask != 0`).

/// Default world geometry (chunks, props, terrain).
pub const RENDER_LAYER_WORLD: u32 = 1 << 0;
/// First-person overlay (separate lens / depth in renderer).
pub const RENDER_LAYER_VIEWMODEL: u32 = 1 << 1;

pub const RENDER_LAYER_DEFAULT_DRAW: u32 = RENDER_LAYER_WORLD;
pub const RENDER_LAYER_WORLD_CAMERA: u32 = RENDER_LAYER_WORLD;
pub const RENDER_LAYER_VIEWMODEL_CAMERA: u32 = RENDER_LAYER_VIEWMODEL;

#[inline]
pub fn render_layer_visible(draw_mask: u32, camera_mask: u32) -> bool {
    draw_mask & camera_mask != 0
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderLayer {
    pub mask: u32,
}

impl RenderLayer {
    pub const fn new(mask: u32) -> Self {
        Self { mask }
    }

    pub const fn world() -> Self {
        Self::new(RENDER_LAYER_WORLD)
    }

    pub const fn viewmodel() -> Self {
        Self::new(RENDER_LAYER_VIEWMODEL)
    }
}
