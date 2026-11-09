//! View frustum tests in clip space (same `view_proj` as mesh shaders).

use glam::{Mat4, Vec3};

use crate::aabb::Aabb;

#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    view_proj: Mat4,
}

impl Frustum {
    pub fn from_view_proj(m: Mat4) -> Self {
        Self { view_proj: m }
    }

    /// True if any part of `aabb` (world space) may intersect the frustum.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        if !aabb.is_valid() {
            return false;
        }
        let corners = aabb_corners(*aabb);
        // Clip half-spaces: -w <= x,y,z <= w (matches glam `perspective_rh` + wgpu clip).
        for axis in 0..3 {
            for negative in [true, false] {
                let mut any_inside = false;
                for c in &corners {
                    let clip = self.view_proj * c.extend(1.0);
                    let w = clip.w;
                    let v = match axis {
                        0 => clip.x,
                        1 => clip.y,
                        _ => clip.z,
                    };
                    let inside = if negative { v >= -w } else { v <= w };
                    if inside {
                        any_inside = true;
                        break;
                    }
                }
                if !any_inside {
                    return false;
                }
            }
        }
        true
    }
}

fn aabb_corners(aabb: Aabb) -> [Vec3; 8] {
    let mn = aabb.min;
    let mx = aabb.max;
    [
        Vec3::new(mn.x, mn.y, mn.z),
        Vec3::new(mx.x, mn.y, mn.z),
        Vec3::new(mn.x, mx.y, mn.z),
        Vec3::new(mx.x, mx.y, mn.z),
        Vec3::new(mn.x, mn.y, mx.z),
        Vec3::new(mx.x, mn.y, mx.z),
        Vec3::new(mn.x, mx.y, mx.z),
        Vec3::new(mx.x, mx.y, mx.z),
    ]
}

#[cfg(test)]
#[path = "frustum_tests.rs"]
mod tests;
