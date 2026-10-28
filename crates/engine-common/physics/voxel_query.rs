//! DDA queries against voxel volumes (no Rapier). Used by the physics backend
//! to merge voxel hits with Rapier hits in `cast_ray` / `cast_capsule`.
//!
//! A voxel volume is a `VOXEL_VOLUME_SIZE`³ grid of unit cells. Cell `(x,y,z)`
//! occupies the world AABB `[origin+(x,y,z), origin+(x+1,y+1,z+1))`. `solid`
//! is a bitset: bit `i = x + y*S + z*S*S` set ⇔ cell is solid.

use super::abi::{VOXEL_VOLUME_BITSET_BYTES, VOXEL_VOLUME_SIZE};
use super::backend::{CapsuleCastHit, RayCastHit};

/// One registered voxel volume (host-side; not a Rapier collider).
#[derive(Clone, Debug)]
pub struct VoxelVolume {
    /// World-space min corner of cell (0,0,0).
    pub origin: [i32; 3],
    /// Solid bitset, bit `i = x + y*S + z*S*S`.
    pub solid: Box<[u8; VOXEL_VOLUME_BITSET_BYTES]>,
}

impl VoxelVolume {
    #[inline]
    pub fn is_solid(&self, x: usize, y: usize, z: usize) -> bool {
        if x >= VOXEL_VOLUME_SIZE || y >= VOXEL_VOLUME_SIZE || z >= VOXEL_VOLUME_SIZE {
            return false;
        }
        let i = x + y * VOXEL_VOLUME_SIZE + z * VOXEL_VOLUME_SIZE * VOXEL_VOLUME_SIZE;
        self.solid[i >> 3] & (1 << (i & 7)) != 0
    }
}

const S: f32 = VOXEL_VOLUME_SIZE as f32;
const S_I: i32 = VOXEL_VOLUME_SIZE as i32;

/// Ray vs voxel volume (Amanatides-Woo DDA). Returns the first solid hit.
///
/// `origin`/`dir` are world-space; `dir` need not be unit (normalized inside).
/// `max_dist` is in world units along the (normalized) direction.
pub fn cast_ray_volume(
    volume: &VoxelVolume,
    origin: [f32; 3],
    dir: [f32; 3],
    max_dist: f32,
    entity: u64,
) -> Option<RayCastHit> {
    let max_dist = max_dist.max(0.0);
    if max_dist < 1e-8 {
        return None;
    }
    let mut d = dir;
    let dlen = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if dlen < 1e-8 {
        return None;
    }
    let inv = 1.0 / dlen;
    d[0] *= inv;
    d[1] *= inv;
    d[2] *= inv;

    let ox = volume.origin[0] as f32;
    let oy = volume.origin[1] as f32;
    let oz = volume.origin[2] as f32;
    // Volume-local float coords (cell units).
    let mut x = origin[0] - ox;
    let mut y = origin[1] - oy;
    let mut z = origin[2] - oz;

    // If outside [0, S)³, advance to the entry point (slab intersection).
    let mut t_enter = 0.0f32;
    let mut t_exit = max_dist;
    let outside = x < 0.0 || x >= S || y < 0.0 || y >= S || z < 0.0 || z >= S;
    if outside {
        for axis in 0..3 {
            let p = [x, y, z][axis];
            let dd = d[axis];
            if dd.abs() < 1e-8 {
                if p < 0.0 || p > S {
                    return None;
                }
            } else {
                let inv_d = 1.0 / dd;
                let t1 = (0.0 - p) * inv_d;
                let t2 = (S - p) * inv_d;
                let (tnear, tfar) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
                t_enter = t_enter.max(tnear);
                t_exit = t_exit.min(tfar);
                if t_enter > t_exit {
                    return None;
                }
            }
        }
        if t_enter > max_dist {
            return None;
        }
        x += d[0] * t_enter;
        y += d[1] * t_enter;
        z += d[2] * t_enter;
    }
    let max_remaining = (t_exit - t_enter).min(max_dist - t_enter).max(0.0);

    let mut ix = x.floor() as i32;
    let mut iy = y.floor() as i32;
    let mut iz = z.floor() as i32;

    let mut step = [0i32; 3];
    let mut t_max = [0.0f32; 3];
    let mut t_delta = [0.0f32; 3];
    let pos = [x, y, z];
    for axis in 0..3 {
        if d[axis] > 0.0 {
            step[axis] = 1;
            let boundary = (ix + 1) as f32;
            t_max[axis] = (boundary - pos[axis]) / d[axis];
        } else if d[axis] < 0.0 {
            step[axis] = -1;
            let boundary = ix as f32;
            t_max[axis] = (boundary - pos[axis]) / d[axis];
        } else {
            step[axis] = 0;
            t_max[axis] = f32::INFINITY;
        }
        t_delta[axis] = if d[axis] != 0.0 { 1.0 / d[axis].abs() } else { f32::INFINITY };
    }

    let mut traveled = 0.0f32;
    let mut last_axis = 0usize;
    loop {
        if (0..S_I).contains(&ix) && (0..S_I).contains(&iy) && (0..S_I).contains(&iz) {
            if volume.is_solid(ix as usize, iy as usize, iz as usize) {
                let mut normal = [0.0f32; 3];
                normal[last_axis] = -step[last_axis] as f32;
                let point = [
                    origin[0] + d[0] * traveled,
                    origin[1] + d[1] * traveled,
                    origin[2] + d[2] * traveled,
                ];
                return Some(RayCastHit {
                    fraction: if max_dist > 0.0 { traveled / max_dist } else { 1.0 },
                    normal,
                    point,
                    entity,
                });
            }
        }
        // Advance to the next cell boundary.
        if t_max[0] < t_max[1] && t_max[0] < t_max[2] {
            if t_max[0] > max_remaining {
                break;
            }
            traveled = t_max[0];
            ix += step[0];
            t_max[0] += t_delta[0];
            last_axis = 0;
        } else if t_max[1] < t_max[2] {
            if t_max[1] > max_remaining {
                break;
            }
            traveled = t_max[1];
            iy += step[1];
            t_max[1] += t_delta[1];
            last_axis = 1;
        } else {
            if t_max[2] > max_remaining {
                break;
            }
            traveled = t_max[2];
            iz += step[2];
            t_max[2] += t_delta[2];
            last_axis = 2;
        }
    }
    None
}

/// Capsule (approximated as its AABB) swept vs a voxel volume. Resolves the
/// first solid cell hit along `dir` (need not be unit) up to `max_dist`,
/// returning the face normal of the contacted cell.
///
/// The capsule is treated as an AABB `[min, max]` (Z-up: `min.z = center.z -
/// half_height`, `max.z = center.z + half_height`, XY inflated by `radius`).
/// This is the standard voxel-mover approximation; for unit-cube terrain the
/// error vs a true capsule is sub-block and visually identical.
pub fn cast_capsule_volume(
    volume: &VoxelVolume,
    center: [f32; 3],
    half_height: f32,
    radius: f32,
    dir: [f32; 3],
    max_dist: f32,
    entity: u64,
) -> Option<CapsuleCastHit> {
    let max_dist = max_dist.max(0.0);
    if max_dist < 1e-8 {
        return None;
    }
    let mut d = dir;
    let dlen = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if dlen < 1e-8 {
        return None;
    }
    let inv = 1.0 / dlen;
    d[0] *= inv;
    d[1] *= inv;
    d[2] *= inv;

    // AABB of the capsule in world space.
    let aabb_min = [
        center[0] - radius,
        center[1] - radius,
        center[2] - half_height,
    ];
    let aabb_max = [
        center[0] + radius,
        center[1] + radius,
        center[2] + half_height,
    ];
    let extent = [
        aabb_max[0] - aabb_min[0],
        aabb_max[1] - aabb_min[1],
        aabb_max[2] - aabb_min[2],
    ];

    let ox = volume.origin[0] as f32;
    let oy = volume.origin[1] as f32;
    let oz = volume.origin[2] as f32;
    let local_min = [aabb_min[0] - ox, aabb_min[1] - oy, aabb_min[2] - oz];

    // start_solid: any solid cell under the starting AABB.
    let x0 = local_min[0].floor() as i32;
    let y0 = local_min[1].floor() as i32;
    let z0 = local_min[2].floor() as i32;
    let x1 = (local_min[0] + extent[0]).floor() as i32;
    let y1 = (local_min[1] + extent[1]).floor() as i32;
    let z1 = (local_min[2] + extent[2]).floor() as i32;
    for cz in z0..=z1 {
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if (0..S_I).contains(&cx)
                    && (0..S_I).contains(&cy)
                    && (0..S_I).contains(&cz)
                    && volume.is_solid(cx as usize, cy as usize, cz as usize)
                {
                    return Some(CapsuleCastHit {
                        toi: 0.0,
                        normal: [0.0, 0.0, 1.0],
                        point: center,
                        start_solid: true,
                        entity,
                    });
                }
            }
        }
    }

    // Swept AABB vs voxel grid: per-axis slab resolution. For each axis, march
    // the leading face through cells and test the swept slab at each step.
    let mut best_toi = max_dist;
    let mut best_normal = [0.0f32; 3];
    let mut hit = false;

    for axis in 0..3 {
        if d[axis].abs() < 1e-8 {
            continue;
        }
        let sign = if d[axis] > 0.0 { 1.0 } else { -1.0 };
        // Leading edge of the AABB along this axis (volume-local).
        let lead = if sign > 0.0 {
            local_min[axis] + extent[axis]
        } else {
            local_min[axis]
        };
        // First cell boundary the leading edge crosses.
        let mut cell = if sign > 0.0 {
            lead.floor() as i32 + 1
        } else {
            lead.floor() as i32
        };
        let mut t_boundary = if sign > 0.0 {
            (cell as f32 - lead) / d[axis]
        } else {
            (cell as f32 - lead) / d[axis]
        };
        let t_step = (1.0 / d[axis].abs()).max(1e-6);
        let (a, b) = other_axes(axis);

        let mut t = 0.0f32;
        while t < best_toi && t < max_dist {
            // AABB of the swept body at parameter t (volume-local).
            let cur_min = [
                local_min[0] + d[0] * t,
                local_min[1] + d[1] * t,
                local_min[2] + d[2] * t,
            ];
            let cur_max = [
                cur_min[0] + extent[0],
                cur_min[1] + extent[1],
                cur_min[2] + extent[2],
            ];
            // The cell the leading face is entering at this t.
            let cell_axis = if sign > 0.0 { cell - 1 } else { cell };
            // Slab of cells overlapping the other two axes.
            let ca0 = cur_min[a].floor() as i32;
            let ca1 = (cur_max[a] - 1e-6).floor() as i32;
            let cb0 = cur_min[b].floor() as i32;
            let cb1 = (cur_max[b] - 1e-6).floor() as i32;
            for cb in cb0..=cb1 {
                for ca in ca0..=ca1 {
                    let (cx, cy, cz) = match axis {
                        0 => (cell_axis, ca, cb),
                        1 => (ca, cell_axis, cb),
                        _ => (ca, cb, cell_axis),
                    };
                    if (0..S_I).contains(&cx)
                        && (0..S_I).contains(&cy)
                        && (0..S_I).contains(&cz)
                        && volume.is_solid(cx as usize, cy as usize, cz as usize)
                    {
                        let mut n = [0.0f32; 3];
                        n[axis] = -sign;
                        if t < best_toi {
                            best_toi = t;
                            best_normal = n;
                            hit = true;
                        }
                        break;
                    }
                }
                if hit {
                    break;
                }
            }
            if hit {
                break;
            }
            t = t_boundary;
            cell += sign as i32;
            t_boundary += t_step;
        }
    }

    if !hit {
        return None;
    }
    let point = [
        center[0] + d[0] * best_toi,
        center[1] + d[1] * best_toi,
        center[2] + d[2] * best_toi,
    ];
    Some(CapsuleCastHit {
        toi: best_toi,
        normal: best_normal,
        point,
        start_solid: false,
        entity,
    })
}

#[inline]
fn other_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}
