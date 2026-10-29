//! Forward frame: [`crate::render_graph::RenderGraph::execute_on_encoder`] + pass dispatch.

use std::collections::HashMap;
use std::ffi::c_void;

use crate::frame::context::FrameCtx;
use crate::frame::schedule::res;
use crate::gpu::Renderer;
use crate::render_graph::NodeContext;
use crate::render_profile::{GraphNodeTiming, lap_ms};

/// Per-frame payload in [`NodeContext::user_data`] (raw pointers avoid graph + pass borrow split).
pub struct GraphFrameUserData {
    pub renderer: *mut Renderer,
    pub ctx: *mut FrameCtx<'static>,
    pub sprite_pipeline: *const wgpu::RenderPipeline,
    pub hud_vp: [[f32; 4]; 4],
}

fn push_graph_timing(ctx: &mut FrameCtx<'_>, name: &str, lap_start: std::time::Instant, skipped: bool) {
    let ms = lap_start.elapsed().as_secs_f32() * 1000.0;
    ctx.prof.graph_nodes.push(GraphNodeTiming {
        name: name.to_string(),
        ms,
        skipped,
    });
}

pub fn dispatch_forward_node(nc: &mut NodeContext<'_>) {
    if nc.user_data.is_null() {
        return;
    }
    let state = unsafe { &mut *(nc.user_data as *mut GraphFrameUserData) };
    let renderer = unsafe { &mut *state.renderer };
    let ctx = unsafe { &mut *state.ctx };
    let sprite_pipeline = unsafe { &*state.sprite_pipeline };
    let hud_vp = state.hud_vp;

    let name = nc.node_name;
    let lap_start = *ctx.lap;
    let mut skipped = false;

    match name {
        "upload_prep" => renderer.pass_upload_prep(ctx),
        "sky" => {
            if ctx.snapshot.sky.is_some() {
                renderer.pass_sky_and_clouds(ctx, nc.encoder);
            } else {
                skipped = true;
                let _ = lap_ms(ctx.lap);
            }
        }
        "clear_hdr" => {
            if ctx.snapshot.sky.is_none() {
                renderer.pass_clear_hdr_no_sky(ctx, nc.encoder);
            } else {
                skipped = true;
                let _ = lap_ms(ctx.lap);
            }
        }
        "sprites_3d" => renderer.pass_sprites_3d(ctx, nc.encoder, sprite_pipeline),
        "mesh" => renderer.pass_mesh(ctx, nc.encoder),
        "viewmodel" => renderer.pass_viewmodel(ctx, nc.encoder),
        "terrain_splat" => renderer.pass_terrain_splat(ctx, nc.encoder),
        "debug" => renderer.pass_debug_lines(ctx, nc.encoder),
        "hud" => renderer.pass_hud(ctx, nc.encoder, sprite_pipeline, hud_vp),
        "luminance" => {
            skipped = true;
            let _ = lap_ms(ctx.lap);
        }
        "tonemap_ui" => renderer.pass_tonemap_and_ui(ctx, nc.encoder),
        _ => {
            skipped = true;
            let _ = lap_ms(ctx.lap);
        }
    }

    push_graph_timing(ctx, name, lap_start, skipped);
}

/// Run the persistent forward graph (single frame encoder, external HDR/depth/surface views).
pub fn run_frame_graph(
    r: &mut Renderer,
    ctx: &mut FrameCtx<'_>,
    encoder: &mut wgpu::CommandEncoder,
    sprite_pipeline: &wgpu::RenderPipeline,
    hud_vp: [[f32; 4]; 4],
) {
    ctx.prof.graph_nodes.clear();

    r.ensure_depth_for_frame();

    let mut external: HashMap<crate::render_graph::ResId, wgpu::TextureView> = HashMap::new();
    external.insert(res::HDR, ctx.hdr_view.clone());
    let depth = r
        .depth_view_ref()
        .expect("ensure_depth_for_frame must set depth_view");
    external.insert(res::DEPTH, depth.clone());
    if let Some(surface) = ctx.surface_view {
        external.insert(res::SURFACE, surface.clone());
    }

    // SAFETY: `user` lives for this call; graph nodes only run synchronously on this thread.
    let mut user = GraphFrameUserData {
        renderer: r as *mut Renderer,
        ctx: unsafe { std::mem::transmute(ctx) },
        sprite_pipeline: sprite_pipeline as *const wgpu::RenderPipeline,
        hud_vp,
    };
    let user_ptr = &mut user as *mut GraphFrameUserData as *mut c_void;

    let device = r.device.clone();
    let queue = r.queue.clone();
    r.forward_graph_mut()
        .execute_on_encoder_in_order(
            &device,
            &queue,
            encoder,
            &external,
            user_ptr,
        )
        .unwrap_or_else(|e| panic!("[render-graph] forward frame failed: {e}"));
}

fn format_res_list(ids: &[&str]) -> String {
    if ids.is_empty() {
        "—".to_string()
    } else {
        ids.join("+")
    }
}

/// Lines for `r_speeds` render-graph section (registration queue + I/O + ms).
pub fn format_graph_speeds_lines(
    profile: &crate::render_profile::RenderFrameProfile,
    node_specs: &[(&'static str, Vec<&'static str>, Vec<&'static str>)],
) -> Vec<String> {
    let mut lines = Vec::new();
    if profile.graph_nodes.is_empty() {
        return lines;
    }
    let listed = profile.graph_nodes_cpu_ms();
    lines.push(format!(
        "render graph  {:.2} ms listed  ({:.2} ms encode wall)",
        listed, profile.total_encode_ms
    ));
    for (i, node) in profile.graph_nodes.iter().enumerate() {
        let io = node_specs
            .iter()
            .find(|(n, _, _)| *n == node.name.as_str())
            .map(|(_, ins, outs)| {
                format!(
                    "{} → {}",
                    format_res_list(ins),
                    format_res_list(outs)
                )
            })
            .unwrap_or_else(|| "— → —".into());
        let skip = if node.skipped { " skip" } else { "" };
        lines.push(format!(
            "  {:02} {:<14} {:>6.2} ms  {}{}",
            i + 1,
            node.name,
            node.ms,
            io,
            skip
        ));
    }
    lines
}
