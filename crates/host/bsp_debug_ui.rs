//! Client ImGui panel for `r_bsp_debug`: camera ray → worldspawn face dump.

use deerpunk_engine_common::bsp_debug::{raycast_mesh, raycast_tris, BspFaceDebug, BspWorldDebug};
use deerpunk_engine_common::components::{
    BspWorldDebug as BspWorldDebugComp, CameraComponent, LightmapId, LightstyleTable, Mesh3D,
    TransformComponent, Worldspawn,
};
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::world::World;
use glam::{Vec3, Vec4};

use crate::cvar::CvarSystem;

const LINE_CAP: usize = 512;
/// Face-slot preview max side in UI pixels.
const LM_PREVIEW_PX: f32 = 64.0;
const LM_PREVIEW_GAP: f32 = 8.0;
/// Atlas preview: coarse grid (keeps ImGui under 16-bit vtx limit).
const ATLAS_CELLS: i32 = 48;
const ATLAS_CELL_PX: f32 = 3.0; // 48*3 = 144px side
const ATLAS_LAYER_GAP: f32 = 6.0;

/// Text drawn this frame by [`text_line`] — collected for the Copy button.
thread_local! {
    static DUMP: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Draw face picker when `r_bsp_debug` ≥ 1. Mode 2 adds LM atlas stack + face rect.
pub fn tick(cvar: &CvarSystem, client: &World, server: &World) {
    let mode = cvar.get_usize_or("r_bsp_debug", 0);
    if mode == 0 {
        return;
    }
    DUMP.with(|d| d.borrow_mut().clear());

    let Some((origin, dir)) = camera_ray(client) else {
        draw_window("r_bsp_debug: no camera");
        return;
    };

    let Some((mesh, debug, map_name)) = find_world(client) else {
        draw_window("r_bsp_debug: no worldspawn Mesh3D+BspWorldDebug");
        return;
    };

    let hit = raycast_mesh(mesh, debug, origin, dir, 8192.0);
    let ls = find_lightstyles(server);
    let face_ref = hit.as_ref().map(|h| &h.face);
    // All entity hits along the ray (brush ents included) — the visible face
    // may belong to a brush cube drawn OVER the picked worldspawn face.
    let all_hits = raycast_all_entities(client, origin, dir, 8192.0);

    unsafe {
        let mut open = true;
        imgui::sys::igSetNextWindowSize(
            imgui::sys::ImVec2 { x: 420.0, y: 520.0 },
            imgui::sys::ImGuiCond_FirstUseEver as i32,
        );
        if imgui::sys::igBegin(
            b"r_bsp_debug\0".as_ptr() as *const i8,
            &mut open,
            0,
        ) {
            text_line(&format!("map: {map_name}  mode={mode}"));
            text_line(&format!(
                "cam: ({:.1}, {:.1}, {:.1})",
                origin[0], origin[1], origin[2]
            ));
            text_line(&format!(
                "dir: ({:.3}, {:.3}, {:.3})",
                dir[0], dir[1], dir[2]
            ));
            text_line(&format!(
                "faces={} tris={}  atlas={}x{}",
                debug.faces.len(),
                debug.tri_face.len(),
                debug.atlas_w,
                debug.atlas_h
            ));
            imgui::sys::igSeparator();

            text_line("entity hits (closest first):");
            if all_hits.is_empty() {
                text_line("  none");
            }
            for h in all_hits.iter().take(6) {
                text_line(&format!(
                    "  ent #{:x}{}: t={:.2} lm={}",
                    h.0,
                    if h.3 { " [worldspawn]" } else { "" },
                    h.1,
                    match h.2 {
                        Some(id) => id.to_string(),
                        None => "none".into(),
                    }
                ));
            }
            imgui::sys::igSeparator();

            if let Some(hit) = hit.as_ref() {
                let f = &hit.face;
                text_line(&format!("worldspawn hit t={:.2}", hit.t));
                text_line(&format!(
                    "point: ({:.2}, {:.2}, {:.2})",
                    hit.point[0], hit.point[1], hit.point[2]
                ));
                text_line(&format!("tri: {}", hit.tri));
                text_line(&format!("bsp_face: {}", f.bsp_face));
                text_line(&format!("tex: {} (miptex {})", f.tex_name, f.miptex));
                text_line(&format!(
                    "normal: ({:.3}, {:.3}, {:.3})",
                    f.normal[0], f.normal[1], f.normal[2]
                ));
                text_line(&format!(
                    "verts: {}  tris: {}..+{}",
                    f.num_verts, f.first_tri, f.num_tris
                ));
                text_line(&format!("lightofs: {}", f.lightofs));
                text_line(&format!(
                    "lm: {}x{} atlas=({},{}) tmins=[{},{}]",
                    f.lm_w,
                    f.lm_h,
                    f.lm_atlas_x,
                    f.lm_atlas_y,
                    f.texturemins[0],
                    f.texturemins[1]
                ));
                imgui::sys::igSeparator();
                text_line("styles [slot]: idx  scale  pattern  avgRGB");

                for slot in 0..4 {
                    let style = f.styles[slot];
                    if style == 255 {
                        text_line(&format!("  [{slot}]: unused"));
                        continue;
                    }
                    let si = style as usize;
                    let (scale, pat) = if let Some(table) = ls {
                        let scale = table.scales.get(si).copied().unwrap_or(0.0);
                        let pat = table
                            .patterns
                            .get(si)
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        (scale, pat)
                    } else {
                        (f32::NAN, "?")
                    };
                    let avg = f.style_avg_rgb[slot];
                    text_line(&format!(
                        "  [{slot}]: style={style}  scale={scale:.3}  pat=\"{pat}\"  avg=({},{},{})",
                        avg[0], avg[1], avg[2]
                    ));
                }

                imgui::sys::igSeparator();
                text_line("LM layers (raw BSP, pre-scale):");
                draw_lm_layer_row(f);

                imgui::sys::igSeparator();
                dump_vertex_sample(mesh, debug, hit.tri, f);
            } else {
                text_line("hit: none");
            }

            imgui::sys::igSeparator();
            if imgui::sys::igButton(
                b"Copy all\0".as_ptr() as *const i8,
                imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
            ) {
                let text = DUMP.with(|d| d.borrow().join("\n"));
                if let Ok(cz) = std::ffi::CString::new(text) {
                    imgui::sys::igSetClipboardText(cz.as_ptr());
                }
            }
            imgui::sys::igSameLine(0.0, 8.0);
            text_line("(copies all lines above)");
        }
        imgui::sys::igEnd();

        // Separate window on the right: 4 packed LM atlas layers + face rect.
        if mode >= 2 {
            let io = imgui::sys::igGetIO();
            if !io.is_null() {
                let dw = (*io).DisplaySize.x;
                let dh = (*io).DisplaySize.y;
                let panel_w = ATLAS_CELLS as f32 * ATLAS_CELL_PX + 36.0;
                imgui::sys::igSetNextWindowPos(
                    imgui::sys::ImVec2 {
                        x: (dw - panel_w - 8.0).max(0.0),
                        y: 8.0,
                    },
                    imgui::sys::ImGuiCond_Always as i32,
                    imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
                );
                imgui::sys::igSetNextWindowSize(
                    imgui::sys::ImVec2 {
                        x: panel_w,
                        y: (dh - 16.0).max(200.0),
                    },
                    imgui::sys::ImGuiCond_Always as i32,
                );
            }
            let mut atlas_open = true;
            if imgui::sys::igBegin(
                b"r_bsp_debug atlas\0".as_ptr() as *const i8,
                &mut atlas_open,
                imgui::sys::ImGuiWindowFlags_NoCollapse as i32,
            ) {
                text_line(&format!(
                    "atlas {}x{}  (red = face)",
                    debug.atlas_w, debug.atlas_h
                ));
                imgui::sys::igSeparator();
                draw_atlas_stack(debug, face_ref);
            }
            imgui::sys::igEnd();
        }
    }
}

/// Vertical stack of 4 atlas layers; red rect = current face patch.
/// Uses a coarse cell grid (not 1px) — ImGui 16-bit indices can't hold a full atlas blit.
fn draw_atlas_stack(debug: &BspWorldDebug, face: Option<&BspFaceDebug>) {
    let Some(layers) = debug.atlas_layers.as_ref() else {
        text_line("(no atlas)");
        return;
    };
    let aw = debug.atlas_w;
    let ah = debug.atlas_h;
    if aw == 0 || ah == 0 {
        text_line("(empty atlas)");
        return;
    }

    let cells = ATLAS_CELLS;
    let cell = ATLAS_CELL_PX;
    let disp_w = cells as f32 * cell;
    let disp_h = disp_w * (ah as f32 / aw as f32);
    // Keep square cells in X; Y cell count from aspect.
    let cells_y = ((disp_h / cell).round() as i32).max(1);
    let disp_h = cells_y as f32 * cell;
    let need = (aw as usize)
        .saturating_mul(ah as usize)
        .saturating_mul(4);

    unsafe {
        for layer in 0..4 {
            text_line(&format!("layer {layer}"));
            let mut origin = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
            imgui::sys::igGetCursorScreenPos(&mut origin);
            let dl = imgui::sys::igGetWindowDrawList();
            if dl.is_null() {
                return;
            }

            let px = layers[layer].as_ref();
            if px.len() < need {
                text_line("(bad layer bytes)");
                continue;
            }

            for cy in 0..cells_y {
                let y0_src = (cy as u32 * ah) / cells_y as u32;
                let y1_src = (((cy + 1) as u32 * ah) / cells_y as u32).max(y0_src + 1);
                for cx in 0..cells {
                    let x0_src = (cx as u32 * aw) / cells as u32;
                    let x1_src = (((cx + 1) as u32 * aw) / cells as u32).max(x0_src + 1);
                    let (r, g, b) = avg_rgba_block(px, aw, x0_src, y0_src, x1_src, y1_src);
                    let x0 = origin.x + cx as f32 * cell;
                    let y0 = origin.y + cy as f32 * cell;
                    imgui::sys::ImDrawList_AddRectFilled(
                        dl,
                        imgui::sys::ImVec2 { x: x0, y: y0 },
                        imgui::sys::ImVec2 {
                            x: x0 + cell,
                            y: y0 + cell,
                        },
                        rgba_u32(r, g, b),
                        0.0,
                        0,
                    );
                }
            }

            imgui::sys::ImDrawList_AddRect(
                dl,
                imgui::sys::ImVec2 {
                    x: origin.x - 1.0,
                    y: origin.y - 1.0,
                },
                imgui::sys::ImVec2 {
                    x: origin.x + disp_w + 1.0,
                    y: origin.y + disp_h + 1.0,
                },
                rgba_u32(160, 160, 160),
                0.0,
                0,
                1.0,
            );

            if let Some(f) = face {
                if f.lm_w > 0 && f.lm_h > 0 {
                    let rx0 = origin.x + f.lm_atlas_x as f32 / aw as f32 * disp_w;
                    let ry0 = origin.y + f.lm_atlas_y as f32 / ah as f32 * disp_h;
                    let rx1 = origin.x + (f.lm_atlas_x + f.lm_w) as f32 / aw as f32 * disp_w;
                    let ry1 = origin.y + (f.lm_atlas_y + f.lm_h) as f32 / ah as f32 * disp_h;
                    imgui::sys::ImDrawList_AddRect(
                        dl,
                        imgui::sys::ImVec2 { x: rx0, y: ry0 },
                        imgui::sys::ImVec2 { x: rx1, y: ry1 },
                        rgba_u32(255, 32, 32),
                        0.0,
                        0,
                        2.0,
                    );
                }
            }

            imgui::sys::igDummy(imgui::sys::ImVec2 {
                x: disp_w + 2.0,
                y: disp_h + ATLAS_LAYER_GAP,
            });
        }
    }
}

fn avg_rgba_block(px: &[u8], aw: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> (u8, u8, u8) {
    let mut sr = 0u64;
    let mut sg = 0u64;
    let mut sb = 0u64;
    let mut n = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * aw + x) * 4) as usize;
            sr += px[i] as u64;
            sg += px[i + 1] as u64;
            sb += px[i + 2] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return (0, 0, 0);
    }
    ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8)
}

/// Horizontal row of 4 face LM patches (CPU RGB), scaled to ~[`LM_PREVIEW_PX`].
fn draw_lm_layer_row(f: &BspFaceDebug) {
    let w = f.lm_w;
    let h = f.lm_h;
    if w == 0 || h == 0 {
        text_line("(no lightmap)");
        return;
    }
    let scale = (LM_PREVIEW_PX / w.max(h) as f32).max(1.0);
    let pw = w as f32 * scale;
    let ph = h as f32 * scale;

    unsafe {
        let mut origin = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
        imgui::sys::igGetCursorScreenPos(&mut origin);
        let dl = imgui::sys::igGetWindowDrawList();
        if dl.is_null() {
            return;
        }

        for slot in 0..4 {
            let x0 = origin.x + slot as f32 * (pw + LM_PREVIEW_GAP);
            let y0 = origin.y;
            imgui::sys::ImDrawList_AddRect(
                dl,
                imgui::sys::ImVec2 {
                    x: x0 - 1.0,
                    y: y0 - 1.0,
                },
                imgui::sys::ImVec2 {
                    x: x0 + pw + 1.0,
                    y: y0 + ph + 1.0,
                },
                rgba_u32(180, 180, 180),
                0.0,
                0,
                1.0,
            );

            if f.styles[slot] == 255 {
                imgui::sys::ImDrawList_AddRectFilled(
                    dl,
                    imgui::sys::ImVec2 { x: x0, y: y0 },
                    imgui::sys::ImVec2 {
                        x: x0 + pw,
                        y: y0 + ph,
                    },
                    rgba_u32(40, 40, 40),
                    0.0,
                    0,
                );
                continue;
            }

            let Some(rgb) = f.layers_rgb[slot].as_ref() else {
                imgui::sys::ImDrawList_AddRectFilled(
                    dl,
                    imgui::sys::ImVec2 { x: x0, y: y0 },
                    imgui::sys::ImVec2 {
                        x: x0 + pw,
                        y: y0 + ph,
                    },
                    rgba_u32(255, 0, 255),
                    0.0,
                    0,
                );
                continue;
            };

            let need = (w as usize)
                .saturating_mul(h as usize)
                .saturating_mul(3);
            if rgb.len() < need {
                continue;
            }
            for ly in 0..h {
                for lx in 0..w {
                    let i = ((ly * w + lx) * 3) as usize;
                    let col = rgba_u32(rgb[i], rgb[i + 1], rgb[i + 2]);
                    let x1 = x0 + lx as f32 * scale;
                    let y1 = y0 + ly as f32 * scale;
                    imgui::sys::ImDrawList_AddRectFilled(
                        dl,
                        imgui::sys::ImVec2 { x: x1, y: y1 },
                        imgui::sys::ImVec2 {
                            x: x1 + scale,
                            y: y1 + scale,
                        },
                        col,
                        0.0,
                        0,
                    );
                }
            }
        }

        imgui::sys::igDummy(imgui::sys::ImVec2 {
            x: 4.0 * pw + 3.0 * LM_PREVIEW_GAP,
            y: ph + 2.0,
        });
    }
}

fn rgba_u32(r: u8, g: u8, b: u8) -> u32 {
    (255u32 << 24) | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32)
}

/// Print the REAL vertex attrs the GPU uses for this tri + atlas pixel under lm_uv.
/// If uv lands outside the face rect / on white — mesh attrs are broken.
/// If uv lands on the dark patch — the bug is in GPU bind (fallback white LM).
fn dump_vertex_sample(
    mesh: &deerpunk_engine_common::resource::mesh::Mesh,
    debug: &BspWorldDebug,
    tri: u32,
    f: &BspFaceDebug,
) {
    let ti = tri as usize * 3;
    let Some(&vi) = mesh.indices.get(ti) else {
        text_line("vertex dump: bad tri index");
        return;
    };
    let vi = vi as usize;
    let Some(uv) = mesh.lightmap_uvs.get(vi) else {
        text_line("vertex dump: no lm_uv attr");
        return;
    };
    let st = mesh
        .lightmap_styles
        .get(vi)
        .copied()
        .unwrap_or([255.0; 4]);
    text_line(&format!(
        "v0 lm_uv=({:.4},{:.4}) styles=({},{},{},{})",
        uv[0], uv[1], st[0], st[1], st[2], st[3]
    ));
    if debug.atlas_w == 0 || debug.atlas_layers.is_none() {
        return;
    }
    let px = uv[0] * debug.atlas_w as f32;
    let py = uv[1] * debug.atlas_h as f32;
    text_line(&format!(
        "atlas px=({:.1},{:.1}) face rect=[{},{}]+{}x{}",
        px, py, f.lm_atlas_x, f.lm_atlas_y, f.lm_w, f.lm_h
    ));
    let in_rect =
        px >= f.lm_atlas_x as f32 && px < (f.lm_atlas_x + f.lm_w) as f32 && py >= f.lm_atlas_y as f32
            && py < (f.lm_atlas_y + f.lm_h) as f32;
    text_line(&format!("uv inside face rect: {in_rect}"));
    let Some(layers) = debug.atlas_layers.as_ref() else {
        return;
    };
    let ix = (px as u32).min(debug.atlas_w - 1) as usize;
    let iy = (py as u32).min(debug.atlas_h - 1) as usize;
    for layer in 0..4 {
        let p = layers[layer].as_ref();
        let i = (iy * debug.atlas_w as usize + ix) * 4;
        if p.len() >= i + 3 {
            text_line(&format!(
                "  layer{layer} @uv = ({},{},{})",
                p[i],
                p[i + 1],
                p[i + 2]
            ));
        }
    }
}

fn camera_ray(world: &World) -> Option<([f32; 3], [f32; 3])> {
    let t_id = world.component_init::<TransformComponent>();
    let cam_id = world.component_init::<CameraComponent>();
    for e in world.inspect_entities() {
        if !world.has(e.id, t_id) || !world.has(e.id, cam_id) {
            continue;
        }
        let t = unsafe { &*(world.get_raw(e.id, t_id) as *const TransformComponent) };
        let forward = t.rotation * Vec3::Y;
        let len = forward.length();
        if len < 1e-6 {
            continue;
        }
        let d = forward / len;
        return Some((
            [t.position.x, t.position.y, t.position.z],
            [d.x, d.y, d.z],
        ));
    }
    None
}

fn find_world(
    world: &World,
) -> Option<(
    &deerpunk_engine_common::resource::mesh::Mesh,
    &BspWorldDebug,
    String,
)> {
    let mesh_id = world.component_init::<Mesh3D>();
    let dbg_id = world.component_init::<BspWorldDebugComp>();
    let ws_id = world.component_init::<Worldspawn>();
    for e in world.inspect_entities() {
        if !world.has(e.id, mesh_id) || !world.has(e.id, dbg_id) {
            continue;
        }
        let mesh3d = unsafe { &*(world.get_raw(e.id, mesh_id) as *const Mesh3D) };
        let debug = unsafe { &*(world.get_raw(e.id, dbg_id) as *const BspWorldDebugComp) };
        let mesh = mesh3d.mesh.as_deref()?;
        if debug.tri_face.is_empty() {
            continue;
        }
        let map_name = if world.has(e.id, ws_id) {
            let ws = unsafe { &*(world.get_raw(e.id, ws_id) as *const Worldspawn) };
            ws.map_str().to_string()
        } else {
            "?".into()
        };
        return Some((mesh, debug, map_name));
    }
    None
}

fn find_lightstyles(world: &World) -> Option<&LightstyleTable> {
    let ls_id = world.component_init::<LightstyleTable>();
    for e in world.inspect_entities() {
        if !world.has(e.id, ls_id) {
            continue;
        }
        return Some(unsafe { &*(world.get_raw(e.id, ls_id) as *const LightstyleTable) });
    }
    None
}

/// `(ent, t, lightmap_id, is_worldspawn)` for every Mesh3D the ray crosses.
/// Brush ents drawn over the wall are the usual reason a picked face looks wrong.
fn raycast_all_entities(
    world: &World,
    origin: [f32; 3],
    dir: [f32; 3],
    max_t: f32,
) -> Vec<(u64, f32, Option<u32>, bool)> {
    let mesh_id = world.component_init::<Mesh3D>();
    let t_id = world.component_init::<TransformComponent>();
    let lm_id = world.component_init::<LightmapId>();
    let ws_id = world.component_init::<Worldspawn>();

    let mut out = Vec::new();
    for e in world.inspect_entities() {
        if !world.has(e.id, mesh_id) || !world.has(e.id, t_id) {
            continue;
        }
        let mesh3d = unsafe { &*(world.get_raw(e.id, mesh_id) as *const Mesh3D) };
        let Some(mesh) = mesh3d.mesh.as_deref() else {
            continue;
        };
        let t = unsafe { &*(world.get_raw(e.id, t_id) as *const TransformComponent) };
        let lm = if world.has(e.id, lm_id) {
            let l = unsafe { &*(world.get_raw(e.id, lm_id) as *const LightmapId) };
            Some(l.id)
        } else {
            None
        };
        let is_ws = world.has(e.id, ws_id);

        // World→local ray via inverse model matrix.
        let model = glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
        let inv = model.inverse();
        let o_g = Vec3::from(origin);
        let d_g = Vec3::from(dir);
        let o_l = inv.transform_point3(o_g);
        let d_l = inv.transform_point3(o_g + d_g) - o_l;
        let local_t = raycast_tris(mesh, o_l.to_array(), d_l.to_array(), max_t);
        let Some((t_local, _tri)) = local_t else {
            continue;
        };
        // Convert t back to world space (d_l may not be unit length).
        let hit_l = o_l + d_l * t_local;
        let hit_w = model.transform_point3(hit_l);
        let t_world = (hit_w - o_g).dot(d_g);
        out.push((e.id, t_world, lm, is_ws));
    }
    out.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn draw_window(msg: &str) {
    unsafe {
        let mut open = true;
        if imgui::sys::igBegin(
            b"r_bsp_debug\0".as_ptr() as *const i8,
            &mut open,
            0,
        ) {
            text_line(msg);
        }
        imgui::sys::igEnd();
    }
}

fn text_line(s: &str) {
    DUMP.with(|d| d.borrow_mut().push(s.to_string()));
    let mut buf = [0u8; LINE_CAP];
    let n = s.as_bytes().len().min(LINE_CAP - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    unsafe {
        imgui::sys::igTextUnformatted(buf.as_ptr() as *const i8, std::ptr::null());
    }
}
