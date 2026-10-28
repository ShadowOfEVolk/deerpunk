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
                    let inside = if negative {
                        v >= -w
                    } else {
                        v <= w
                    };
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

/// World-space AABB that encloses the camera frustum (for spatial broad-phase).
pub fn frustum_world_aabb(view: Mat4, near: f32, far: f32, fov_y: f32, aspect: f32) -> Aabb {
    let inv_view = view.inverse();
    let tan = (fov_y * 0.5).tan();
    let near_d = near.max(0.05);
    let far_d = far.max(near_d + 0.1);
    let mut out = Aabb::EMPTY;
    let eye = inv_view.transform_point3(Vec3::ZERO);
    out = out.merge(Aabb::from_min_max(eye, eye));
    for d in [near_d, far_d] {
        let h = d * tan;
        let w = h * aspect;
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                // glam `look_to_rh`: camera looks down -Z in view space.
                let v = Vec3::new(sx * w, sy * h, -d);
                let p = inv_view.transform_point3(v);
                out = out.merge(Aabb::from_min_max(p, p));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "frustum_tests.rs"]
mod tests;
