//! Nuklear overlay: GPU mesh pool usage (`r_showalloc`).

use deerpunk_engine_renderer::mesh_alloc_debug::{build_report, index_unit_bytes, SideView};
use deerpunk_engine_renderer::Renderer;

const BAR_H: f32 = 14.0;
const BAR_GAP: f32 = 4.0;

/// `mode`: 0 = off (caller should skip). `1` = all allocators; `N>1` = allocator id `N-1`.
pub fn tick(mode: i32, renderer: &Renderer) {
    if mode <= 0 {
        return;
    }
    let filter = if mode > 1 {
        Some((mode - 1) as u32)
    } else {
        None
    };
    let report = build_report(renderer.mesh_buffer_pool(), renderer.mesh_gpu_pool(), filter);


    let rows = 2 + report.buffers.len() * 4;
    let h = 40.0 + rows as f32 * 20.0;
    if !deerpunk_engine_renderer::host_ui::begin_panel(
        "r_showalloc - mesh GPU pools",
        8.0,
        220.0,
        560.0,
        h.max(160.0),
    ) {
        return;
    }
    deerpunk_engine_renderer::host_ui::label(&format!(
        "live meshes: {}   (green = free, orange = live)",
        report.live_mesh_count
    ));
    if mode > 1 {
        deerpunk_engine_renderer::host_ui::label(&format!("filter: allocator_id={}", mode - 1));
    }
    if report.buffers.is_empty() {
        deerpunk_engine_renderer::host_ui::label("(no mesh pool buffers yet)");
        deerpunk_engine_renderer::host_ui::end_window();
        return;
    }
    for buf in &report.buffers {
        deerpunk_engine_renderer::host_ui::label(&format!(
            "buf={} alloc={} stride={} B",
            buf.buffer_id, buf.allocator_id, buf.stride
        ));
        draw_side_row("vertices", &buf.verts, buf.stride);
        draw_side_row("indices", &buf.indices, index_unit_bytes());
    }
    deerpunk_engine_renderer::host_ui::end_window();
}

fn draw_side_row(label: &str, side: &SideView, unit_bytes: u32) {
    let used = side.used_units();
    let pct = if side.cap > 0 { used as f32 / side.cap as f32 * 100.0 } else { 0.0 };
    deerpunk_engine_renderer::host_ui::label(&format!(
        "{label}: {}/{} ({:.1}%)  segs={}  ~{:.1} KiB / {:.1} KiB",
        used, side.cap, pct, side.used.len(),
        used as f32 * unit_bytes as f32 / 1024.0,
        side.cap as f32 * unit_bytes as f32 / 1024.0,
    ));
    let mut spans = Vec::new();
    spans.push((0.0, 1.0, [28, 28, 32, 255]));
    for &(off, len) in &free_intervals(side) {
        if side.cap == 0 || len == 0 { continue; }
        let a = off as f32 / side.cap as f32;
        let b = (off + len) as f32 / side.cap as f32;
        spans.push((a, b, [55, 130, 75, 255]));
    }
    for &(off, len) in &side.used {
        if side.cap == 0 || len == 0 { continue; }
        let a = off as f32 / side.cap as f32;
        let b = (off + len) as f32 / side.cap as f32;
        spans.push((a, b, [210, 120, 45, 255]));
    }
    deerpunk_engine_renderer::host_ui::bar(&spans);
}

fn free_intervals(side: &SideView) -> Vec<(u32, u32)> {
    let mut free = Vec::new();
    let mut pos = 0u32;
    for &(off, len) in &side.used {
        if off > pos {
            free.push((pos, off - pos));
        }
        pos = off.saturating_add(len);
    }
    if pos < side.cap {
        free.push((pos, side.cap - pos));
    }
    free
}
