//! CPU cost of static mesh batching: plan fingerprint, MDI cache probe, indirect commands.
//!
//! ```text
//! cargo bench -p deerpunk-engine-renderer --bench mesh_batch
//! ```

use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
use deerpunk_engine_common::MeshTexture;
use deerpunk_engine_renderer::mesh_allocator::MeshHandle as GpuMesh;
use deerpunk_engine_renderer::mesh_batch_bench::{
    bench_batch_fingerprint, bench_build_indirect, bench_mdi_cache_probes, StaticBatchPlanBench,
};
use deerpunk_engine_renderer::snapshot::MeshDraw;
use deerpunk_engine_renderer::Aabb;
use std::hint::black_box;
use std::time::Instant;

const DRAWS: usize = 4096;

fn draw(slot: u32) -> MeshDraw {
    MeshDraw {
        mesh: MeshHandle::from_parts(slot + 1, 1),
        model_mat: {
            let mut m = [[0.0; 4]; 4];
            m[3] = [slot as f32, (slot % 64) as f32, 0.0, 1.0];
            m
        },
        layer_mask: 1,
        batchable: true,
        material_id: slot % 8,
        texture: MeshTexture(slot % 4),
        normal: MeshTexture(0),
        orm: MeshTexture(0),
        sun_dir: [0.0; 4],
        ambient: [0.0; 4],
        shader_id: if slot % 32 == 0 { 2 } else { 1 },
        lightmap_gpu: 0,
        first_index: 0,
        index_count: 36,
        face: (slot % 7) as u8,
        world_bounds: Aabb::EMPTY,
        entity_id: slot as u64,
        skip_vis: false,
        uniform_aux: 0,
    }
}

fn gpu(i: usize) -> GpuMesh {
    GpuMesh {
        buffer_id: (i % 4) as u32 + 1,
        vert_offset: (i as u32) * 64,
        vert_count: 24,
        idx_offset: (i as u32) * 36,
        idx_count: 36,
        stride: 32,
    }
}

fn time(label: &str, iters: u32, mut body: impl FnMut() -> u64) {
    let start = Instant::now();
    let mut acc = 0u64;
    for _ in 0..iters {
        acc = acc.wrapping_add(black_box(body()));
    }
    let ns = start.elapsed().as_nanos() / u128::from(iters);
    let per_draw = ns / DRAWS as u128;
    println!("{label}: {ns} ns/frame  {per_draw} ns/draw  (n={DRAWS}, sink={acc})");
}

fn main() {
    let draws: Vec<MeshDraw> = (0..DRAWS as u32).map(draw).collect();
    let gpu_handles: Vec<GpuMesh> = (0..DRAWS).map(gpu).collect();
    let hits: Vec<(usize, u64, usize, u64)> = draws
        .iter()
        .map(|md| (md.mesh.0, 1u64, md.mesh.0, 1u64))
        .collect();
    let misses: Vec<(usize, u64, usize, u64)> = draws
        .iter()
        .enumerate()
        .map(|(i, md)| (draws[(i + 1) % DRAWS].mesh.0, 1u64, md.mesh.0, 1u64))
        .collect();
    let mut order = Vec::with_capacity(DRAWS);
    let mut scratch = Vec::with_capacity(DRAWS);

    println!("static mesh batch bookkeeping, {DRAWS} draws");
    time("fingerprint", 400, || {
        bench_batch_fingerprint(black_box(&draws))
    });
    time("cache hit (same mesh, rev 1)", 400, || {
        u64::from(bench_mdi_cache_probes(black_box(&hits)))
    });
    time("cache miss (draw slot reused)", 400, || {
        u64::from(bench_mdi_cache_probes(black_box(&misses)))
    });
    time("indirect group+write", 200, || {
        u64::from(bench_build_indirect(
            black_box(&draws),
            black_box(&gpu_handles),
            &mut order,
            &mut scratch,
        ))
    });

    let mut plan = StaticBatchPlanBench::new();
    time("static_batch_plan cold", 40, || {
        plan = StaticBatchPlanBench::new();
        plan.frame(black_box(&draws), [0.0, 0.0, 0.0])
    });
    time("static_batch_plan warm", 200, || {
        let t = black_box(plan.frame(black_box(&draws), [1.0, 2.0, 3.0]));
        t
    });
}
