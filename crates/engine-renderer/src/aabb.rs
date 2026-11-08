//! Axis-aligned bounds (world / local mesh space).

use glam::{Mat4, Quat, Vec3};

/// Inclusive min / max corners. `is_valid()` is false when empty.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn from_min_max(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn is_valid(self) -> bool {
        self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z
    }

    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn extent(self) -> Vec3 {
        (self.max - self.min).max(Vec3::ZERO)
    }

    pub fn merge(self, other: Self) -> Self {
        if !self.is_valid() {
            return other;
        }
        if !other.is_valid() {
            return self;
        }
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// World AABB of this box under translation, rotation, and per-axis scale.
    ///
    /// No rotation: `position + scale * min/max` (six multiplies). With rotation the
    /// box stays axis-aligned: center and extents, absolute value of the 3×3.
    pub fn transformed_trs(self, position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        if !self.is_valid() {
            return Self::EMPTY;
        }
        let scaled_min = self.min * scale;
        let scaled_max = self.max * scale;
        let min = scaled_min.min(scaled_max);
        let max = scaled_min.max(scaled_max);
        if rotation == Quat::IDENTITY {
            return Self {
                min: position + min,
                max: position + max,
            };
        }
        let center = (min + max) * 0.5;
        let ext = (max - min) * 0.5;
        let m = glam::Mat3::from_quat(rotation);
        let world_center = m * center + position;
        let world_ext = m.x_axis.abs() * ext.x + m.y_axis.abs() * ext.y + m.z_axis.abs() * ext.z;
        Self {
            min: world_center - world_ext,
            max: world_center + world_ext,
        }
    }

    /// Transform eight corners by `model` and merge.
    pub fn transform(self, model: Mat4) -> Self {
        if !self.is_valid() {
            return Self::EMPTY;
        }
        let corners = [
            Vec3::new(self.min.x, self.min.y, self.min.z),
            Vec3::new(self.max.x, self.min.y, self.min.z),
            Vec3::new(self.min.x, self.max.y, self.min.z),
            Vec3::new(self.max.x, self.max.y, self.min.z),
            Vec3::new(self.min.x, self.min.y, self.max.z),
            Vec3::new(self.max.x, self.min.y, self.max.z),
            Vec3::new(self.min.x, self.max.y, self.max.z),
            Vec3::new(self.max.x, self.max.y, self.max.z),
        ];
        let mut out = Self::EMPTY;
        for c in corners {
            let w = model.transform_point3(c);
            out = out.merge(Self::from_min_max(w, w));
        }
        out
    }

    pub fn intersects(self, other: &Aabb) -> bool {
        if !self.is_valid() || !other.is_valid() {
            return false;
        }
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    pub fn inflate(self, margin: f32) -> Self {
        if !self.is_valid() {
            return self;
        }
        Self {
            min: self.min - Vec3::splat(margin),
            max: self.max + Vec3::splat(margin),
        }
    }

    /// True if this box fully contains `other` (both must be valid).
    pub fn contains_aabb(self, other: &Aabb) -> bool {
        if !self.is_valid() || !other.is_valid() {
            return false;
        }
        self.min.x <= other.min.x
            && self.min.y <= other.min.y
            && self.min.z <= other.min.z
            && self.max.x >= other.max.x
            && self.max.y >= other.max.y
            && self.max.z >= other.max.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_rotation_is_scale_from_corner() {
        let local = Aabb::from_min_max(Vec3::ZERO, Vec3::new(16.0, 16.0, 16.0));
        let world = local.transformed_trs(
            Vec3::new(32.0, 0.0, 8.0),
            Quat::IDENTITY,
            Vec3::new(2.0, 1.0, 0.5),
        );
        assert_eq!(world.min, Vec3::new(32.0, 0.0, 8.0));
        assert_eq!(world.max, Vec3::new(64.0, 16.0, 16.0));
    }

    #[test]
    fn yaw_90_swaps_extents() {
        let local = Aabb::from_min_max(Vec3::new(-1.0, -2.0, 0.0), Vec3::new(1.0, 2.0, 4.0));
        let world = local.transformed_trs(
            Vec3::ZERO,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::ONE,
        );
        let corners = local.transform(glam::Mat4::from_quat(Quat::from_rotation_z(
            std::f32::consts::FRAC_PI_2,
        )));
        assert!((world.min - corners.min).length() < 1e-4);
        assert!((world.max - corners.max).length() < 1e-4);
    }
}
