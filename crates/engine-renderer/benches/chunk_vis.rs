//! Chunk visibility before submit: clip-corner `check_vis` vs one plane set per frame.
//!
//! Submit is a mock that only folds the accepted chunk into a sink.
//!
//! ```text
//! cargo bench -p deerpunk-engine-renderer --bench chunk_vis
//! ```

use deerpunk_engine_common::coord::{perspective_rh_wgpu, view_matrix_z_up_rh};
use deerpunk_engine_renderer::{Aabb, Frustum};
use glam::{Mat4, Quat, Vec3, Vec4};
use std::hint::black_box;
use std::time::Instant;

const CHUNK: f32 = 16.0;
const HALF: f32 = 8.0;
const FRAMES: u32 = 300;

struct Sink {
    n: u64,
    mix: u64,
}

fn mock_submit(sink: &mut Sink, entity: u64, origin: [f32; 3]) {
    sink.n += 1;
    sink.mix ^= entity.wrapping_mul(origin[0].to_bits() as u64 | 1);
}

/// Same work as `ClientEngine::check_vis`: rebuild the frustum and test 8 corners.
fn check_vis_corners(min: [f32; 3], max: [f32; 3], view_proj: Mat4) -> bool {
    let aabb = Aabb::from_min_max(Vec3::from(min), Vec3::from(max));
    Frustum::from_view_proj(view_proj).intersects_aabb(&aabb)
}

fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
    let r0 = view_proj.row(0);
    let r1 = view_proj.row(1);
    let r2 = view_proj.row(2);
    let r3 = view_proj.row(3);
    // Clip test is `-w <= x,y,z <= w`, the same half-spaces as `Frustum::intersects_aabb`.
    [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 + r2, r3 - r2]
}

fn cube_in_planes(planes: &[Vec4; 6], origin: Vec3) -> bool {
    let p = (origin + Vec3::splat(HALF)).extend(1.0);
    for plane in planes {
        let r = HALF * (plane.x.abs() + plane.y.abs() + plane.z.abs());
        if plane.dot(p) < -r {
            return false;
        }
    }
    true
}

fn scan_corners(origins: &[[f32; 3]], view_proj: Mat4, check: fn([f32; 3], [f32; 3], Mat4) -> bool) -> u64 {
    let mut sink = Sink { n: 0, mix: 0 };
    for (i, origin) in origins.iter().enumerate() {
        let max = [origin[0] + CHUNK, origin[1] + CHUNK, origin[2] + CHUNK];
        if !check(*origin, max, view_proj) {
            continue;
        }
        mock_submit(&mut sink, i as u64, *origin);
    }
    black_box(sink.mix);
    sink.n
}

fn scan_planes(origins: &[[f32; 3]], planes: &[Vec4; 6]) -> u64 {
    let mut sink = Sink { n: 0, mix: 0 };
    for (i, origin) in origins.iter().enumerate() {
        if !cube_in_planes(planes, Vec3::from(*origin)) {
            continue;
        }
        mock_submit(&mut sink, i as u64, *origin);
    }
    black_box(sink.mix);
    sink.n
}

fn time(label: &str, chunks: usize, mut body: impl FnMut() -> u64) {
    let start = Instant::now();
    let mut acc = 0u64;
    for _ in 0..FRAMES {
        acc = acc.wrapping_add(black_box(body()));
    }
    let ns = start.elapsed().as_nanos() / u128::from(FRAMES);
    let per = ns / chunks as u128;
    println!("{label}: {ns} ns/frame  {per} ns/chunk  (chunks={chunks}, submits={acc})");
}

fn main() {
    let eye = Vec3::new(8.0, 8.0, 48.0);
    let view = view_matrix_z_up_rh(eye, Quat::IDENTITY);
    let proj = perspective_rh_wgpu(70.0_f32.to_radians(), 16.0 / 9.0, 0.1, 256.0);
    let view_proj = proj * view;
    let planes = frustum_planes(view_proj);

    let mut origins = Vec::new();
    for z in -1..6 {
        for y in -12..13 {
            for x in -12..13 {
                origins.push([x as f32 * CHUNK, y as f32 * CHUNK, z as f32 * CHUNK]);
            }
        }
    }

    let mut mismatch = 0u32;
    let mut visible = 0u32;
    for origin in &origins {
        let max = [origin[0] + CHUNK, origin[1] + CHUNK, origin[2] + CHUNK];
        let a = check_vis_corners(*origin, max, view_proj);
        let b = cube_in_planes(&planes, Vec3::from(*origin));
        mismatch += u32::from(a != b);
        visible += u32::from(a);
    }
    println!(
        "chunk cubes {}  visible {}  plane/corner mismatches {}",
        origins.len(),
        visible,
        mismatch
    );
    if mismatch != 0 {
        std::process::exit(1);
    }

    let check: fn([f32; 3], [f32; 3], Mat4) -> bool = black_box(check_vis_corners);
    time("check_vis corners + mock submit", origins.len(), || {
        scan_corners(black_box(&origins), view_proj, check)
    });
    time("6 planes once + mock submit", origins.len(), || {
        scan_planes(black_box(&origins), black_box(&planes))
    });
}
