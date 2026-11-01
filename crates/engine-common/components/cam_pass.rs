//! Per-camera render pass toggles (Bevy-style: which phases run for this view).
//!
//! Generic bitmask — the engine does NOT name game-specific passes like "viewmodel".
//! Games pick a [`u32`] mask of these bits per camera. `pass_mask` replaces the old
//! `features: u8` + implicit `FrameGraphScope` (Primary/Stack) split: a camera declares
//! exactly the passes it wants, and the render graph runs only those nodes for it.

pub const SKY: u32 = 1 << 0;
pub const SPRITES_3D: u32 = 1 << 1;
pub const MESH: u32 = 1 << 2;
pub const TERRAIN: u32 = 1 << 3;
pub const DEBUG: u32 = 1 << 4;

/// Typical primary world view: sky + 3D sprites + mesh + terrain + debug.
pub const DEFAULT_WORLD: u32 = SKY | SPRITES_3D | MESH | TERRAIN | DEBUG;

/// Overlay/weapon view: mesh only (drawn on top of the world HDR, own depth).
pub const DEFAULT_OVERLAY: u32 = MESH;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_world_includes_all_scene_passes() {
        assert_ne!(DEFAULT_WORLD & SKY, 0);
        assert_ne!(DEFAULT_WORLD & SPRITES_3D, 0);
        assert_ne!(DEFAULT_WORLD & MESH, 0);
        assert_ne!(DEFAULT_WORLD & TERRAIN, 0);
        assert_ne!(DEFAULT_WORLD & DEBUG, 0);
    }

    #[test]
    fn overlay_is_mesh_only() {
        assert_eq!(DEFAULT_OVERLAY, MESH);
        assert_eq!(DEFAULT_OVERLAY & DEFAULT_WORLD, MESH);
    }

    #[test]
    fn pass_bits_are_disjoint() {
        let all = SKY | SPRITES_3D | MESH | TERRAIN | DEBUG;
        assert_eq!(all.count_ones(), 5);
    }
}
