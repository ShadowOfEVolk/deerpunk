//! BSP face pick debug (r_bsp_debug): face meta + raycast.

use std::sync::Arc;

use crate::resource::mesh::Mesh;

/// One world face after mesh build (for picker UI).
#[derive(Clone, Debug)]
pub struct BspFaceDebug {
    /// Index into BSP faces lump (world model range).
    pub bsp_face: u32,
    pub tex_name: String,
    pub miptex: i32,
    pub styles: [u8; 4],
    pub lightofs: i32,
    pub lm_w: u32,
    pub lm_h: u32,
    pub lm_atlas_x: u32,
    pub lm_atlas_y: u32,
    pub texturemins: [i32; 2],
    /// Average RGB of each style layer (255,255,255 if unused).
    pub style_avg_rgb: [[u8; 3]; 4],
    /// Raw LM RGB (`lm_w*lm_h*3`) per style slot — for debug preview.
    pub layers_rgb: [Option<Arc<[u8]>>; 4],
    pub normal: [f32; 3],
    pub num_verts: u32,
    pub first_tri: u32,
    pub num_tris: u32,
}

/// Worldspawn debug sidecar (tri → face + face table + LM atlas CPU copy).
#[derive(Clone, Debug, Default)]
pub struct BspWorldDebug {
    pub faces: Arc<Vec<BspFaceDebug>>,
    /// `indices.len()/3` entries → index into `faces`.
    pub tri_face: Arc<Vec<u32>>,
    /// Packed LM atlas size (0 if none).
    pub atlas_w: u32,
    pub atlas_h: u32,
    /// RGBA8 pixels per array layer (same W×H), for `r_bsp_debug 2`.
    pub atlas_layers: Option<[Arc<[u8]>; 4]>,
}

impl BspWorldDebug {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn face_for_tri(&self, tri: u32) -> Option<&BspFaceDebug> {
        let fi = *self.tri_face.get(tri as usize)?;
        self.faces.get(fi as usize)
    }
}

/// Hit result from camera ray vs world mesh.
#[derive(Clone, Debug)]
pub struct BspPickHit {
    pub t: f32,
    pub point: [f32; 3],
    pub tri: u32,
    pub face: BspFaceDebug,
}

/// Closest triangle hit along ray (Möller–Trumbore). `max_t` caps distance.
pub fn raycast_mesh(
    mesh: &Mesh,
    debug: &BspWorldDebug,
    origin: [f32; 3],
    dir: [f32; 3],
    max_t: f32,
) -> Option<BspPickHit> {
    let (t, tri) = raycast_tris(mesh, origin, dir, max_t)?;
    let face = debug.face_for_tri(tri)?.clone();
    let d = normalize3(dir);
    let point = [
        origin[0] + d[0] * t,
        origin[1] + d[1] * t,
        origin[2] + d[2] * t,
    ];
    Some(BspPickHit { t, point, tri, face })
}

/// Raw tri raycast without face meta (brush entities). Returns `(t, tri)`.
pub fn raycast_tris(
    mesh: &Mesh,
    origin: [f32; 3],
    dir: [f32; 3],
    max_t: f32,
) -> Option<(f32, u32)> {
    let dir_len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if dir_len < 1e-8 {
        return None;
    }
    let inv = 1.0 / dir_len;
    let d = [dir[0] * inv, dir[1] * inv, dir[2] * inv];
    let o = origin;

    let mut best_t = max_t;
    let mut best_tri: Option<u32> = None;

    let ntri = mesh.indices.len() / 3;
    for ti in 0..ntri {
        let i0 = mesh.indices[ti * 3] as usize;
        let i1 = mesh.indices[ti * 3 + 1] as usize;
        let i2 = mesh.indices[ti * 3 + 2] as usize;
        let Some(v0) = mesh.positions.get(i0).copied() else {
            continue;
        };
        let Some(v1) = mesh.positions.get(i1).copied() else {
            continue;
        };
        let Some(v2) = mesh.positions.get(i2).copied() else {
            continue;
        };
        if let Some(t) = moller_trumbore(o, d, v0, v1, v2) {
            if t > 1e-4 && t < best_t {
                best_t = t;
                best_tri = Some(ti as u32);
            }
        }
    }
    best_tri.map(|tri| (best_t, tri))
}

fn normalize3(dir: [f32; 3]) -> [f32; 3] {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-8 {
        return dir;
    }
    [dir[0] / len, dir[1] / len, dir[2] / len]
}

fn moller_trumbore(
    orig: [f32; 3],
    dir: [f32; 3],
    v0: [f32; 3],
    v1: [f32; 3],
    v2: [f32; 3],
) -> Option<f32> {
    const EPS: f32 = 1e-7;
    let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let p = cross(dir, e2);
    let det = dot(e1, p);
    if det.abs() < EPS {
        return None;
    }
    let inv_det = 1.0 / det;
    let tvec = [orig[0] - v0[0], orig[1] - v0[1], orig[2] - v0[2]];
    let u = dot(tvec, p) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(tvec, e1);
    let v = dot(dir, q) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv_det;
    if t < EPS {
        return None;
    }
    Some(t)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn avg_rgb(rgb: &[u8]) -> [u8; 3] {
    let n = rgb.len() / 3;
    if n == 0 {
        return [0, 0, 0];
    }
    let mut s = [0u64; 3];
    for i in 0..n {
        s[0] += rgb[i * 3] as u64;
        s[1] += rgb[i * 3 + 1] as u64;
        s[2] += rgb[i * 3 + 2] as u64;
    }
    let n = n as u64;
    [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
}
