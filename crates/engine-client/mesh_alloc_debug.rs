//! ImGui overlay: GPU mesh pool usage (`r_showalloc`).

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

    unsafe {
        let mut open = true;
        if !imgui::sys::igBegin(
            b"r_showalloc - mesh GPU pools\0".as_ptr() as *const i8,
            &mut open,
            0,
        ) {
            return;
        }

        text_line(&format!(
            "live meshes: {}   (green = free, orange = live regions; holes from free() are free)",
            report.live_mesh_count
        ));
        if mode > 1 {
            text_line(&format!("filter: allocator_id={}", mode - 1));
        }
        imgui::sys::igSeparator();

        if report.buffers.is_empty() {
            text_line("(no mesh pool buffers yet)");
            imgui::sys::igEnd();
            return;
        }

        for buf in &report.buffers {
            text_line(&format!(
                "buf={} alloc={} stride={} B",
                buf.buffer_id,
                buf.allocator_id,
                buf.stride
            ));
            draw_side_row("vertices", &buf.verts, buf.stride);
            draw_side_row("indices", &buf.indices, index_unit_bytes());
            imgui::sys::igSeparator();
        }

        imgui::sys::igEnd();
    }
}

fn draw_side_row(label: &str, side: &SideView, unit_bytes: u32) {
    let used = side.used_units();
    let pct = if side.cap > 0 {
        used as f32 / side.cap as f32 * 100.0
    } else {
        0.0
    };
    text_line(&format!(
        "{label}: {}/{} ({:.1}%)  segs={}  ~{:.1} KiB / {:.1} KiB",
        used,
        side.cap,
        pct,
        side.used.len(),
        used as f32 * unit_bytes as f32 / 1024.0,
        side.cap as f32 * unit_bytes as f32 / 1024.0,
    ));

    unsafe {
        let mut avail = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
        imgui::sys::igGetContentRegionAvail(&mut avail);
        let w = avail.x.max(120.0);
        let mut origin = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
        imgui::sys::igGetCursorScreenPos(&mut origin);
        let dl = imgui::sys::igGetWindowDrawList();
        if dl.is_null() || side.cap == 0 {
            imgui::sys::igDummy(imgui::sys::ImVec2 { x: w, y: BAR_H });
            return;
        }

        let y1 = origin.y + BAR_H;
        imgui::sys::ImDrawList_AddRectFilled(
            dl,
            origin,
            imgui::sys::ImVec2 {
                x: origin.x + w,
                y: y1,
            },
            rgba_u32(28, 28, 32),
            0.0,
            0,
        );

        let mut free = free_intervals(side);
        for &(off, len) in &free {
            draw_segment(dl, origin.x, origin.y, y1, w, side.cap, off, len, rgba_u32(55, 130, 75));
        }
        for &(off, len) in &side.used {
            draw_segment(dl, origin.x, origin.y, y1, w, side.cap, off, len, rgba_u32(210, 120, 45));
        }

        imgui::sys::ImDrawList_AddRect(
            dl,
            imgui::sys::ImVec2 {
                x: origin.x - 1.0,
                y: origin.y - 1.0,
            },
            imgui::sys::ImVec2 {
                x: origin.x + w + 1.0,
                y: y1 + 1.0,
            },
            rgba_u32(140, 140, 150),
            0.0,
            0,
            1.0,
        );

        imgui::sys::igDummy(imgui::sys::ImVec2 {
            x: w,
            y: BAR_H + BAR_GAP,
        });
    }
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

unsafe fn draw_segment(
    dl: *mut imgui::sys::ImDrawList,
    ox: f32,
    oy: f32,
    y1: f32,
    bar_w: f32,
    cap: u32,
    off: u32,
    len: u32,
    col: u32,
) {
    if cap == 0 || len == 0 {
        return;
    }
    let x0 = ox + (off as f32 / cap as f32) * bar_w;
    let x1 = ox + ((off + len) as f32 / cap as f32) * bar_w;
    if x1 <= x0 {
        return;
    }
    imgui::sys::ImDrawList_AddRectFilled(
        dl,
        imgui::sys::ImVec2 { x: x0, y: oy },
        imgui::sys::ImVec2 { x: x1, y: y1 },
        col,
        0.0,
        0,
    );
}

fn text_line(s: &str) {
    let mut buf = [0u8; 512];
    let bytes = s.as_bytes();
    let n = bytes.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&bytes[..n]);
    unsafe {
        imgui::sys::igTextUnformatted(buf.as_ptr() as *const i8, std::ptr::null());
    }
}

fn rgba_u32(r: u8, g: u8, b: u8) -> u32 {
    (255u32 << 24) | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32)
}
