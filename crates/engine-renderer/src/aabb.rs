//! Axis-aligned bounds (world / local mesh space).

use glam::{Mat4, Vec3};

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

    /// Tight AABB from mesh vertex positions (object space).
    pub fn from_positions(positions: &[[f32; 3]]) -> Self {
        if positions.is_empty() {
            return Self::EMPTY;
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for p in positions {
            let v = Vec3::from(*p);
            min = min.min(v);
            max = max.max(v);
        }
        Self { min, max }
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
