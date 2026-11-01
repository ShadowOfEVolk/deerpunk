use super::cam_pass;
use super::render_layer::RENDER_LAYER_DEFAULT_CAMERA;

/// Opaque depth-target identifier. The engine pools depth textures by this id:
/// cameras sharing the same id share one depth buffer; different ids get separate
/// depth buffers. Games define their own constants (e.g. world vs overlay) — the
/// engine never interprets the value.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DepthTargetId(pub u32);

/// How a camera loads its color attachment at the start of its passes.
///
/// The clear *color* itself is scene-level ([`crate::snapshot::RenderScene::clear`],
/// typically driven by the sky compositor); this enum is only the per-camera
/// decision to clear vs. keep (overlay/stacked cameras keep prior color).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorLoad {
    /// Clear the color target to the scene clear color before drawing.
    Clear,
    /// Keep existing color (overlay / stacked cameras draw on top of prior passes).
    Load,
}

/// How a camera loads its depth attachment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepthLoad {
    /// Clear depth to `value` (typically `1.0`) before drawing.
    Clear(f32),
    /// Keep existing depth (test against depth written by a previous camera on the
    /// same [`DepthTargetId`], without resetting it).
    Load,
    /// No depth attachment at all (depth test/write disabled for this camera).
    None,
}

/// Per-camera depth attachment ownership + load policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthAttachment {
    /// Which depth pool slot this camera uses.
    pub target: DepthTargetId,
    pub load: DepthLoad,
}

impl DepthAttachment {
    /// Fresh depth buffer for this camera (most common case).
    pub const fn owned(target: DepthTargetId) -> Self {
        Self { target, load: DepthLoad::Clear(1.0) }
    }
}

/// Camera lens. View pose comes from [`TransformComponent`] (+ [`ParentComponent`] for pawns).
#[derive(Clone, Copy, Debug)]
pub struct CameraComponent {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    /// Draw mask on [`RenderLayer`] must overlap this field (bitwise AND).
    pub layer_mask: u32,
    /// Sort key for multi-camera surface compose (lower first).
    pub order: u8,
    /// [`cam_pass`] bitmask — which render passes this camera runs.
    pub pass_mask: u32,
    /// Color attachment load policy for this camera.
    pub color_load: ColorLoad,
    /// Depth attachment (target id + load policy).
    pub depth: DepthAttachment,
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self {
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 1024.0,
            layer_mask: RENDER_LAYER_DEFAULT_CAMERA,
            order: 0,
            pass_mask: cam_pass::DEFAULT_WORLD,
            color_load: ColorLoad::Clear,
            depth: DepthAttachment::owned(DepthTargetId(0)),
        }
    }
}

impl CameraComponent {
    pub fn aspect_ratio(&self, width: f32, height: f32) -> f32 {
        width / height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::cam_pass;

    #[test]
    fn default_lens_has_default_layer_and_world_passes() {
        let c = CameraComponent::default();
        assert_eq!(c.layer_mask, RENDER_LAYER_DEFAULT_CAMERA);
        assert_eq!(c.order, 0);
        assert_eq!(c.pass_mask, cam_pass::DEFAULT_WORLD);
        assert_eq!(c.depth.load, DepthLoad::Clear(1.0));
        assert_eq!(c.color_load, ColorLoad::Clear);
    }

    #[test]
    fn owned_depth_is_clear_one() {
        let d = DepthAttachment::owned(DepthTargetId(7));
        assert_eq!(d.target, DepthTargetId(7));
        assert_eq!(d.load, DepthLoad::Clear(1.0));
    }
}
