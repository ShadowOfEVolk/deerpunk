use crate::engine_api::engine_api::EngineApiMethod;
use deerpunk_engine_common::bsp_debug::{raycast_mesh, BspWorldDebug};
use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::resource::mesh::Mesh;
use std::ffi::c_void;
use std::sync::Arc;

// ── World trampolines ─────────────────────────────────────────

pub extern "C" fn tramp_get(engine: *mut u8, entity: u64, comp: u64) -> *mut c_void {
    let host = unsafe { &*(engine as *const crate::Host) };
    let ptr = host.server_engine.world.get_raw(entity, comp);
    if ptr.is_null() {
        std::ptr::null_mut()
    } else {
        ptr as *mut c_void
    }
}

pub extern "C" fn tramp_set(
    engine: *mut u8,
    entity: u64,
    comp: u64,
    data: *const c_void,
    size: usize,
) {
    println!("set: entity={}, comp={}, size={}", entity, comp, size);
    let host = unsafe { &*(engine as *const crate::Host) };
    host.server_engine.world.set_raw(entity, comp, data, size);
}

pub extern "C" fn tramp_has(engine: *mut u8, entity: u64, comp: u64) -> bool {
    let host = unsafe { &*(engine as *const crate::Host) };
    host.server_engine.world.has(entity, comp)
}

pub extern "C" fn tramp_remove(engine: *mut u8, entity: u64, comp: u64) {
    let host = unsafe { &*(engine as *const crate::Host) };
    host.server_engine.world.remove(entity, comp);
}

pub extern "C" fn tramp_spawn(engine: *mut u8) -> u64 {
    let host = unsafe { &*(engine as *const crate::Host) };
    host.server_engine.world.spawn()
}

pub extern "C" fn tramp_destroy(engine: *mut u8, entity: u64) {
    let host = unsafe { &*(engine as *const crate::Host) };
    host.server_engine.world.destroy(entity);
}

pub extern "C" fn tramp_clear_world(engine: *mut u8) {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let world = &host.server_engine.world;
    let ids: Vec<u64> = world.inspect_entities().into_iter().map(|e| e.id).collect();
    log::info!("[world] clear — destroying {} entities", ids.len());
    for id in ids {
        world.destroy(id);
    }
    // Flush deferred deletes so the next place_bsp cannot leave a second world mesh.
    world.simulate(0.0);
}

pub extern "C" fn tramp_spawn_bundle(
    _engine: *mut u8,
    _comp_ids: *const u64,
    _comp_data: *const *mut c_void,
    _count: usize,
) -> u64 {
    // TODO: implement spawn_bundle
    0
}

pub extern "C" fn tramp_query(
    engine: *mut u8,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
    out: *mut u64,
    max: usize,
) -> usize {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let require = unsafe { std::slice::from_raw_parts(require, require_n) };
    let _exclude = unsafe { std::slice::from_raw_parts(exclude, exclude_n) };
    // Exclude terms not wired yet — match require-only (current gamelib usage).
    let qid = host.server_engine.world.query_prepare_ids(require);
    let mut ids = Vec::new();
    if let Some(batches) = host.server_engine.world.query_batches(qid) {
        for batch in batches {
            let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
            ids.extend_from_slice(entities);
        }
    }
    host.server_engine.world.query_release(qid);
    let mut written = 0usize;
    for &e in &ids {
        if written >= max || out.is_null() {
            break;
        }
        unsafe {
            *out.add(written) = e;
        }
        written += 1;
    }
    written
}

pub extern "C" fn tramp_query_count(
    engine: *mut u8,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
) -> usize {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let require = unsafe { std::slice::from_raw_parts(require, require_n) };
    let _exclude = unsafe { std::slice::from_raw_parts(exclude, exclude_n) };
    let qid = host.server_engine.world.query_prepare_ids(require);
    let mut count = 0usize;
    if let Some(batches) = host.server_engine.world.query_batches(qid) {
        for batch in batches {
            count += batch.count;
        }
    }
    host.server_engine.world.query_release(qid);
    count
}

pub extern "C" fn tramp_query_matches(
    engine: *mut u8,
    entity: u64,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
) -> bool {
    let host = unsafe { &*(engine as *const crate::Host) };
    let require = unsafe { std::slice::from_raw_parts(require, require_n) };
    let exclude = unsafe { std::slice::from_raw_parts(exclude, exclude_n) };
    for &c in require {
        if !host.server_engine.world.has(entity, c) {
            return false;
        }
    }
    for &c in exclude {
        if host.server_engine.world.has(entity, c) {
            return false;
        }
    }
    true
}

pub extern "C" fn tramp_query_prepare(
    engine: *mut u8,
    expr_ptr: *const u8,
    expr_len: usize,
) -> u32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let expr = unsafe {
        let bytes = std::slice::from_raw_parts(expr_ptr, expr_len);
        std::str::from_utf8_unchecked(bytes)
    };
    host.server_engine.world.query_prepare(expr) as u32
}

pub extern "C" fn tramp_query_execute(
    engine: *mut u8,
    query_id: u32,
    callback: extern "C" fn(u64, *const *const c_void, usize, *mut c_void),
    ctx: *mut c_void,
) {
    let host = unsafe { &*(engine as *const crate::Host) };
    let qid = query_id as usize;
    if let Some(batches) = host.server_engine.world.query_batches(qid) {
        for batch in batches {
            let entities = unsafe {
                std::slice::from_raw_parts(batch.entities, batch.count)
            };
            let field_count = batch.columns.len();
            for (i, &entity) in entities.iter().enumerate() {
                let fields: Vec<*const c_void> = (0..field_count)
                    .map(|f| unsafe { batch.columns[f].add(i * batch.sizes[f]) })
                    .collect();
                callback(entity, fields.as_ptr(), field_count, ctx);
            }
        }
    }
}

pub extern "C" fn tramp_free_value(_engine: *mut u8, _ptr: *mut c_void) {
    // get returns a pointer to Flecs internal storage — do NOT free it
}

pub extern "C" fn tramp_component_init_raw(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    size: usize,
    align: usize,
) -> u64 {
    let host = unsafe { &*(engine as *const crate::Host) };
    let name = unsafe {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len);
        std::str::from_utf8_unchecked(bytes)
    };
    let desc = crate::world::ComponentDescriptor {
        name: name.to_string(),
        size,
        align,
    };
    host.server_engine.world.component_init_raw(&desc)
}

// ── Trace line (server world BSP raycast) ────────────────────

/// Result layout for `v1:world:trace_line` (gamelib mirrors this struct).
#[repr(C)]
pub struct TraceLineResult {
    /// 1 = hit, 0 = miss.
    pub hit: u32,
    /// Distance fraction along the ray (0..1), 1.0 on miss.
    pub fraction: f32,
    /// World-space hit point (origin on miss).
    pub point: [f32; 3],
    /// NUL-padded texture name of the hit face.
    pub tex: [u8; 64],
}

/// `v1:world:trace_line(ox, oy, oz, ex, ey, ez, out_ptr) -> 1 hit / 0 miss`.
///
/// Raycasts against the placed BSP world mesh in the server world.
#[allow(clippy::too_many_arguments)]
pub extern "C" fn tramp_trace_line(
    engine: *mut u8,
    ox: f32,
    oy: f32,
    oz: f32,
    ex: f32,
    ey: f32,
    ez: f32,
    out_ptr: *mut TraceLineResult,
) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let world = &mut host.server_engine.world;

    // Find the BSP world entity (BspWorldDebug + Mesh3D).
    let dbg_id = world.component_init::<BspWorldDebug>();
    let mesh_id = world.component_init::<Mesh3D>();
    let q = world.query_prepare_ids(&[dbg_id, mesh_id]);
    let mut found: Option<(Arc<Mesh>, BspWorldDebug)> = None;
    if let Some(batches) = world.query_batches(q) {
        'outer: for batch in batches {
            let count = batch.count;
            let dbgs = unsafe {
                std::slice::from_raw_parts(batch.columns[0] as *const BspWorldDebug, count)
            };
            let meshes = unsafe {
                std::slice::from_raw_parts(batch.columns[1] as *const Mesh3D, count)
            };
            for (dbg, m) in dbgs.iter().zip(meshes.iter()) {
                if let Some(mesh) = &m.mesh {
                    found = Some((mesh.clone(), dbg.clone()));
                    break 'outer;
                }
            }
        }
    }
    world.query_release(q);

    let origin = [ox, oy, oz];
    let delta = [ex - ox, ey - oy, ez - oz];
    let len = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();

    let hit = found.and_then(|(mesh, debug)| {
        if len < 1e-4 {
            return None;
        }
        let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
        raycast_mesh(&mesh, &debug, origin, dir, len)
    });

    let result = match &hit {
        Some(h) => {
            let mut tex = [0u8; 64];
            let name = h.face.tex_name.as_bytes();
            let n = name.len().min(63);
            tex[..n].copy_from_slice(&name[..n]);
            TraceLineResult {
                hit: 1,
                fraction: if len > 0.0 { h.t / len } else { 0.0 },
                point: h.point,
                tex,
            }
        }
        None => TraceLineResult {
            hit: 0,
            fraction: 1.0,
            point: origin,
            tex: [0u8; 64],
        },
    };
    if !out_ptr.is_null() {
        unsafe { *out_ptr = result };
    }
    if hit.is_some() { 1 } else { 0 }
}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:world:get", callback: tramp_get as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:set", callback: tramp_set as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:has", callback: tramp_has as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:remove", callback: tramp_remove as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:spawn", callback: tramp_spawn as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:destroy", callback: tramp_destroy as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:clear", callback: tramp_clear_world as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:spawn_bundle", callback: tramp_spawn_bundle as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:query", callback: tramp_query as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:query_count", callback: tramp_query_count as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:query_matches", callback: tramp_query_matches as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:free_value", callback: tramp_free_value as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:component_init_raw", callback: tramp_component_init_raw as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:query_prepare", callback: tramp_query_prepare as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:query_execute", callback: tramp_query_execute as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:world:trace_line", callback: tramp_trace_line as *const (), flags: 0 } }
