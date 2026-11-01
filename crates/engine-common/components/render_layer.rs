//! Bitmask: which camera passes may draw this entity (`draw.mask & camera.layer_mask != 0`).
//!
//! The engine only provides the generic bitmask mechanism + a conventional default
//! (bit 0). Games name their own layers (world, viewmodel, ui, …) and assign bits.
//! No game-specific names like "viewmodel" live here.

/// Conventional default layer (bit 0). Games may reuse it or define their own bits.
pub const RENDER_LAYER_DEFAULT: u32 = 1 << 0;

/// Default draw mask = default layer.
pub const RENDER_LAYER_DEFAULT_DRAW: u32 = RENDER_LAYER_DEFAULT;

/// Default camera mask = default layer.
pub const RENDER_LAYER_DEFAULT_CAMERA: u32 = RENDER_LAYER_DEFAULT;

/// Conventional primary scene layer (bit 0). Games may alias this or define their
/// own; the engine treats it only as "the default layer".
pub const RENDER_LAYER_WORLD: u32 = RENDER_LAYER_DEFAULT;
pub const RENDER_LAYER_WORLD_CAMERA: u32 = RENDER_LAYER_DEFAULT;

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

    pub const fn default_layer() -> Self {
        Self::new(RENDER_LAYER_DEFAULT)
    }

    /// Conventional primary scene layer (bit 0).
    pub const fn world() -> Self {
        Self::new(RENDER_LAYER_WORLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_visible_is_bitwise_and() {
        assert!(render_layer_visible(0b001, 0b001));
        assert!(render_layer_visible(0b011, 0b010));
        assert!(!render_layer_visible(0b001, 0b010));
        assert!(!render_layer_visible(0, 0b001));
    }

    #[test]
    fn default_layer_is_single_bit() {
        assert_eq!(RENDER_LAYER_DEFAULT, 1);
        assert_eq!(RENDER_LAYER_DEFAULT_DRAW, RENDER_LAYER_DEFAULT);
        assert_eq!(RENDER_LAYER_DEFAULT_CAMERA, RENDER_LAYER_DEFAULT);
    }
}
