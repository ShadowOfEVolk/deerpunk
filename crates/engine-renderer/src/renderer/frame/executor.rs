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
        ForwardPass::Particles => {
            if !ctx.primary {
                return;
            }
            render_scope!("render/particles");
            r.pass_particles(ctx, encoder);
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
            r.pass_tonemap(ctx, encoder);
        }
        ForwardPass::PostFx => {
            render_scope!("render/post_fx");
            r.pass_postfx(ctx, encoder);
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

/// ASCII render-graph block for `r_speeds`: a file-tree listing of the frame's
/// passes with CPU ms.
///
/// `frame -> cam -> post`: camera passes run per camera in order, `hud` +
/// `tonemap_ui` run once per target after all cameras. Group rows show the
/// phase total. Passes that only ran while masked out (skipped) show `skip`
/// instead of a time. Multiple cameras sum and mark `xN`.
pub fn format_graph_speeds_lines(
    profile: &crate::render_profile::RenderFrameProfile,
) -> Vec<String> {
    use crate::render_graph::ForwardPass;

    const CAM: [ForwardPass; 7] = [
        ForwardPass::UploadPrep,
        ForwardPass::Sky,
        ForwardPass::ClearHdr,
        ForwardPass::Sprites3d,
        ForwardPass::Mesh,
        ForwardPass::Particles,
        ForwardPass::Debug,
    ];
    const POST: [ForwardPass; 3] = [
        ForwardPass::Hud,
        ForwardPass::TonemapUi,
        ForwardPass::PostFx,
    ];

    // Aggregate this frame's timings by pass name (sum ms, runs, any real run).
    let mut acc: std::collections::HashMap<&str, (f32, u32, bool)> =
        std::collections::HashMap::new();
    for node in &profile.graph_nodes {
        let e = acc.entry(node.name.as_str()).or_insert((0.0, 0, false));
        e.0 += node.ms;
        e.1 += 1;
        e.2 |= !node.skipped;
    }

    let cell = |name: &str| -> String {
        match acc.get(name) {
            Some(&(_, runs, true)) if runs > 1 => {
                let ms = acc[name].0;
                format!("{ms:.2}x{runs}")
            }
            Some(&(ms, _, true)) => format!("{ms:.2}"),
            _ => "skip".to_string(),
        }
    };
    let rows = |passes: &[ForwardPass]| -> (Vec<(&'static str, String)>, f32) {
        let rows = passes.iter().map(|p| (p.name(), cell(p.name()))).collect();
        let total = passes
            .iter()
            .map(|p| acc.get(p.name()).map_or(0.0, |e| e.0))
            .sum();
        (rows, total)
    };
    let (cam_rows, cam_total) = rows(&CAM);
    let (post_rows, post_total) = rows(&POST);

    // Uniform columns: widest pass name and widest ms cell.
    let name_w = CAM
        .iter()
        .chain(POST.iter())
        .map(|p| p.name().len())
        .max()
        .unwrap_or(0);
    let ms_w = cam_rows
        .iter()
        .chain(post_rows.iter())
        .map(|(_, c)| c.len())
        .chain([
            format!("{cam_total:.2}").len(),
            format!("{post_total:.2}").len(),
        ])
        .max()
        .unwrap_or(0);

    let branch = |last: bool| if last { "`-- " } else { "|-- " };
    let row =
        |prefix: &str, name: &str, cell: &str| format!("{prefix}{name:<name_w$}  {cell:>ms_w$}");

    let mut lines = Vec::new();
    let listed = profile.graph_nodes_cpu_ms();
    let other = (profile.total_encode_ms - listed).max(0.0);
    lines.push(format!(
        "graph {:.2} ms (+{:.2} outside passes)",
        listed, other
    ));
    if profile.graph_nodes.is_empty() {
        return lines;
    }
    lines.push("frame".to_string());
    lines.push(row(branch(false), "cam", &format!("{cam_total:.2}")));
    for (i, (name, cell)) in cam_rows.iter().enumerate() {
        let last = i + 1 == cam_rows.len();
        lines.push(row(&format!("|   {}", branch(last)), name, cell));
    }
    lines.push(row(branch(true), "post", &format!("{post_total:.2}")));
    for (i, (name, cell)) in post_rows.iter().enumerate() {
        let last = i + 1 == post_rows.len();
        lines.push(row(&format!("    {}", branch(last)), name, cell));
    }
    lines
}

#[cfg(test)]
mod speeds_tests {
    use super::*;
    use crate::render_profile::{GraphNodeTiming, RenderFrameProfile};

    fn node(name: &str, ms: f32, skipped: bool) -> GraphNodeTiming {
        GraphNodeTiming {
            name: name.to_string(),
            ms,
            skipped,
        }
    }

    #[test]
    fn graph_speeds_tree_and_aggregation() {
        let mut p = RenderFrameProfile::default();
        p.graph_nodes = vec![
            node("upload_prep", 0.1, false),
            node("sky", 0.0, true),
            node("mesh", 1.5, false),
            node("mesh", 1.5, false),
            node("hud", 0.2, false),
            node("tonemap_ui", 0.3, false),
            node("post_fx", 0.2, false),
        ];
        p.total_encode_ms = 4.0;
        let lines = format_graph_speeds_lines(&p);
        assert_eq!(lines.len(), 14, "{lines:?}");
        assert!(lines[0].starts_with("graph 3.80 ms"), "{})", lines[0]);
        assert_eq!(lines[1], "frame");
        assert!(lines[2].starts_with("|-- cam"), "{}", lines[2]);
        assert!(lines[2].contains("3.10"), "{}", lines[2]);
        let tree = lines.join("\n");
        assert!(tree.contains("`-- post"), "{tree}");
        assert!(tree.contains("|   `-- debug"), "{tree}");
        assert!(tree.contains("    |-- tonemap_ui"), "{tree}");
        assert!(tree.contains("    `-- post_fx"), "{tree}");
        // Passes in execution order, one per line.
        let order: Vec<usize> = [
            "upload_prep",
            "sky",
            "clear_hdr",
            "sprites_3d",
            "mesh",
            "particles",
            "debug",
            "hud",
            "tonemap_ui",
            "post_fx",
        ]
        .iter()
        .map(|n| tree.find(n).unwrap())
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "passes out of order:\n{tree}"
        );
        assert!(tree.contains("3.00x2"), "{tree}");
        assert!(tree.contains("skip"), "{tree}");
    }
}
