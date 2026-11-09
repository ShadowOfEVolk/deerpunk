//! Forward frame: typed dispatch of [`crate::render_graph::FORWARD_PASSES`].

use crate::frame::context::FrameCtx;
use crate::frame::scope::{self, Phase};
use crate::render_graph::{ForwardPass, FORWARD_PASSES};
use crate::render_profile::{lap_ms, GraphNodeTiming};
use crate::renderer::Renderer;
use crate::snapshot::cam_features;

fn push_graph_timing(
    ctx: &mut FrameCtx<'_>,
    name: &str,
    lap_start: std::time::Instant,
    skipped: bool,
) {
    let ms = lap_start.elapsed().as_secs_f32() * 1000.0;
    ctx.prof.graph_nodes.push(GraphNodeTiming {
        name: name.to_string(),
        ms,
        skipped,
    });
}

/// Execute one forward pass against the renderer / frame context directly.
fn run_forward_pass(
    pass: ForwardPass,
    r: &mut Renderer,
    ctx: &mut FrameCtx<'_>,
    encoder: &mut wgpu::CommandEncoder,
    sprite_pipeline: &wgpu::RenderPipeline,
    hud_vp: [[f32; 4]; 4],
    phase: Phase,
) {
    let name = pass.name();
    let lap_start = *ctx.lap;
    let mut skipped = false;

    if !scope::node_enabled(pass, phase) {
        skipped = true;
        let _ = lap_ms(ctx.lap);
        push_graph_timing(ctx, name, lap_start, skipped);
        return;
    }

    macro_rules! render_scope {
        ($label:literal) => {
            #[cfg(any(feature = "tracy", feature = "puffin"))]
            let _render_profile =
                crate::profile_active().then(|| crate::gamelib_profile::zone_scope($label));
        };
    }

    match pass {
        ForwardPass::UploadPrep => {
            render_scope!("render/upload_prep");
            r.pass_upload_prep(ctx);
        }
        ForwardPass::Sky => {
            render_scope!("render/sky");
            if r.draws_sky()
                && ctx.snapshot.pass_mask & cam_features::SKY != 0
                && ctx.snapshot.sky.is_some()
            {
                r.pass_sky_and_clouds(ctx, encoder);
            } else {
                skipped = true;
                let _ = lap_ms(ctx.lap);
            }
        }
        ForwardPass::ClearHdr => {
            render_scope!("render/clear_hdr");
            // Clear HDR only when this camera declares a color clear AND sky did not
            // already clear (sky pass clears to the scene clear color itself).
            let clears_color =
                ctx.snapshot.color_load == deerpunk_engine_common::components::ColorLoad::Clear;
            let sky_cleared = r.draws_sky()
                && ctx.snapshot.pass_mask & cam_features::SKY != 0
                && ctx.snapshot.sky.is_some();
            if clears_color && !sky_cleared {
                r.pass_clear_hdr_no_sky(ctx, encoder);
            } else {
                skipped = true;
                let _ = lap_ms(ctx.lap);
            }
        }
        ForwardPass::Sprites3d => {
            render_scope!("render/sprites_3d");
            r.pass_sprites_3d(ctx, encoder, sprite_pipeline);
        }
        ForwardPass::Mesh => {
            render_scope!("render/mesh");
            r.pass_mesh(ctx, encoder);
        }
        ForwardPass::Debug => {
            render_scope!("render/debug");
            if ctx.snapshot.pass_mask & cam_features::DEBUG != 0 {
                r.pass_debug_lines(ctx, encoder);
            } else {
                skipped = true;
                let _ = lap_ms(ctx.lap);
            }
        }
        ForwardPass::Hud => {
            render_scope!("render/hud");
            r.pass_hud(ctx, encoder, sprite_pipeline, hud_vp);
        }
        ForwardPass::TonemapUi => {
            render_scope!("render/tonemap_ui");
            r.pass_tonemap_and_ui(ctx, encoder);
        }
    }

    push_graph_timing(ctx, name, lap_start, skipped);
}

/// Run the forward pass list in declaration order on the frame encoder
/// (HDR / surface views come from `FrameCtx`).
pub fn run_frame_graph(
    r: &mut Renderer,
    ctx: &mut FrameCtx<'_>,
    encoder: &mut wgpu::CommandEncoder,
    sprite_pipeline: &wgpu::RenderPipeline,
    hud_vp: [[f32; 4]; 4],
    phase: Phase,
) {
    // Depth is owned by the per-camera depth pool, not the graph. Passes fetch their
    // depth view from `Renderer::ensure_depth(snapshot.depth.target, w, h)`.
    for pass in FORWARD_PASSES {
        run_forward_pass(pass, r, ctx, encoder, sprite_pipeline, hud_vp, phase);
    }
}

fn format_res_list(ids: &[&str]) -> String {
    if ids.is_empty() {
        "—".to_string()
    } else {
        ids.join("+")
    }
}

/// Lines for `r_speeds` render-graph section (pass queue + I/O + ms).
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
            .map(|(_, ins, outs)| format!("{} → {}", format_res_list(ins), format_res_list(outs)))
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
