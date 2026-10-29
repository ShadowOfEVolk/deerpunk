//! Per-phase host timings for the inspector `systems` collection (full frame).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_renderer::RenderFrameProfile;

struct PhaseWindow {
    sum_us: u64,
    last_us: u64,
}

struct ProfilerState {
    /// Accumulator for the current ~1s window.
    phases: HashMap<&'static str, PhaseWindow>,
    frame_count: u32,
    frame_wall_sum_us: u64,
    frame_wall_last_us: u64,
    period_start: Instant,
    /// Last published window (what `systems.list` returns).
    published: Value,
    #[cfg(any(feature = "client", feature = "full"))]
    last_render: Option<RenderFrameProfile>,
}

impl ProfilerState {
    fn new() -> Self {
        Self {
            phases: HashMap::new(),
            frame_count: 0,
            frame_wall_sum_us: 0,
            frame_wall_last_us: 0,
            period_start: Instant::now(),
            published: empty_snapshot(),
            #[cfg(any(feature = "client", feature = "full"))]
            last_render: None,
        }
    }
}

static PROFILER: LazyLock<Mutex<ProfilerState>> =
    LazyLock::new(|| Mutex::new(ProfilerState::new()));

thread_local! {
    static FRAME_START: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

fn empty_snapshot() -> Value {
    json!({
        "frame_count": 0,
        "period_secs": 0.0,
        "summary": {
            "frame_ms_last": 0.0,
            "frame_ms_avg": 0.0,
            "fps_avg": 0.0,
        },
        "count": 0,
        "phases": [],
        "systems": [],
        "render": null,
    })
}

fn with_profiler<R>(f: impl FnOnce(&mut ProfilerState) -> R) -> R {
    let mut g = PROFILER
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f(&mut g)
}

fn record_phase(state: &mut ProfilerState, name: &'static str, elapsed: Duration) {
    let micros = elapsed.as_micros().min(u128::from(u32::MAX)) as u64;
    let slot = state.phases.entry(name).or_insert(PhaseWindow {
        sum_us: 0,
        last_us: 0,
    });
    slot.sum_us += micros;
    slot.last_us = micros;
}

/// Mark start of a full host frame (wall-clock), including render.
pub fn begin_frame() {
    FRAME_START.set(Some(Instant::now()));
}

/// Record one named phase duration for the current frame.
pub fn record(name: &'static str, elapsed: Duration) {
    with_profiler(|state| record_phase(state, name, elapsed));
}

/// Record a duration given in milliseconds (e.g. render profile).
pub fn record_ms(name: &'static str, ms: f32) {
    if ms > 0.0 {
        record(name, Duration::from_secs_f32(ms / 1000.0));
    }
}

/// Attach GPU/encode breakdown from the client renderer (last completed frame).
#[cfg(any(feature = "client", feature = "full"))]
pub fn ingest_render_profile(profile: &RenderFrameProfile) {
    with_profiler(|state| {
        state.last_render = Some(profile.clone());
        // Phase timings come from host scopes (`frame.render`, `frame.extract`, …).
        // `RenderFrameProfile` is detail for inspector `render` + `r_speeds` parity.
    });
}

/// RAII timer for a named phase.
pub struct Scope {
    name: &'static str,
    start: Instant,
}

impl Scope {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start: Instant::now(),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        record(self.name, self.start.elapsed());
    }
}

#[inline]
pub fn scope(name: &'static str) -> Scope {
    Scope::new(name)
}

fn round_ms(ms: f64) -> f64 {
    (ms * 100.0).round() / 100.0
}

fn round_pct(p: f64) -> f64 {
    (p * 10.0).round() / 10.0
}

fn build_snapshot(state: &ProfilerState) -> Value {
    let frames = u64::from(state.frame_count.max(1));
    let period_secs = state.period_start.elapsed().as_secs_f32();
    let frame_wall_avg_us = state.frame_wall_sum_us / frames;
    let frame_ms_avg = (frame_wall_avg_us as f64) / 1000.0;
    let frame_ms_last = (state.frame_wall_last_us as f64) / 1000.0;
    let fps_avg = if frame_ms_avg > 0.0 {
        1000.0 / frame_ms_avg
    } else {
        0.0
    };

    let scoped_sum_us: u64 = state.phases.values().map(|p| p.sum_us).sum();
    let unaccounted_sum_us = state.frame_wall_sum_us.saturating_sub(scoped_sum_us);

    let mut phase_rows: Vec<Value> = state
        .phases
        .iter()
        .map(|(name, win)| {
            let avg_us = win.sum_us / frames;
            let last_us = win.last_us;
            let percent = if state.frame_wall_sum_us > 0 {
                (win.sum_us as f64 / state.frame_wall_sum_us as f64) * 100.0
            } else {
                0.0
            };
            json!({
                "name": name,
                "time_ms_avg": round_ms((avg_us as f64) / 1000.0),
                "time_ms_last": round_ms((last_us as f64) / 1000.0),
                "time_us_avg": avg_us,
                "percent": round_pct(percent),
            })
        })
        .collect();

    if unaccounted_sum_us > 0 {
        let last_unaccounted = state
            .frame_wall_last_us
            .saturating_sub(state.phases.values().map(|p| p.last_us).sum());
        phase_rows.push(json!({
            "name": "frame.unaccounted",
            "time_ms_avg": round_ms((unaccounted_sum_us as f64 / frames as f64) / 1000.0),
            "time_ms_last": round_ms((last_unaccounted as f64) / 1000.0),
            "time_us_avg": unaccounted_sum_us / frames,
            "percent": round_pct(
                (unaccounted_sum_us as f64 / state.frame_wall_sum_us as f64) * 100.0,
            ),
        }));
    }

    phase_rows.sort_by(|a, b| {
        b.get("time_ms_avg")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .partial_cmp(&a.get("time_ms_avg").and_then(|v| v.as_f64()).unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let phases_indexed: Vec<Value> = phase_rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut o = row.as_object().cloned().unwrap_or_default();
            o.insert("index".into(), json!(i));
            Value::Object(o)
        })
        .collect();

    #[cfg(any(feature = "client", feature = "full"))]
    let render = state.last_render.as_ref().map(|p| {
        let r_speeds_ms = p.extract_ms + p.total_encode_ms;
        json!({
            "r_speeds_ms": round_ms(r_speeds_ms as f64),
            "extract_ms": round_ms(p.extract_ms as f64),
            "prep_ms": round_ms(p.prep_ms as f64),
            "encode_ms": round_ms(p.total_encode_ms as f64),
            "game_frame_ui_ms": round_ms(p.game_frame_ui_ms as f64),
            "surface_acquire_ms": round_ms(p.surface_acquire_ms as f64),
            "after_imgui_ms": round_ms(p.after_imgui_ms as f64),
            "encoder_finish_ms": round_ms(p.encoder_finish_ms as f64),
            "queue_submit_ms": round_ms(p.queue_submit_ms as f64),
            "after_submit_ms": round_ms(p.after_submit_ms as f64),
            "after_present_ms": round_ms(p.after_present_ms as f64),
            "after_encode_ms": round_ms(p.after_encode_ms as f64),
            "width": p.width,
            "height": p.height,
            "mesh_draws": p.mesh_draws,
            "graph_nodes": p.graph_nodes.iter().map(|node| {
                json!({
                    "name": node.name,
                    "ms": round_ms(node.ms as f64),
                    "skipped": node.skipped,
                })
            }).collect::<Vec<_>>(),
        })
    });

    #[cfg(not(any(feature = "client", feature = "full")))]
    let render: Option<Value> = None;

    json!({
        "frame_count": state.frame_count,
        "period_secs": period_secs,
        "summary": {
            "frame_ms_last": round_ms(frame_ms_last),
            "frame_ms_avg": round_ms(frame_ms_avg),
            "fps_avg": round_ms(fps_avg),
        },
        "count": phases_indexed.len(),
        "phases": phases_indexed,
        "systems": phases_indexed,
        "render": render,
    })
}

/// Finish the frame; publish a window snapshot every ~1s.
pub fn end_frame() {
    let frame_us = FRAME_START
        .get()
        .map(|start| start.elapsed().as_micros().min(u128::from(u32::MAX)) as u64)
        .unwrap_or(0);
    FRAME_START.set(None);

    with_profiler(|state| {
        state.frame_count = state.frame_count.saturating_add(1);
        state.frame_wall_sum_us += frame_us;
        state.frame_wall_last_us = frame_us;

        if state.period_start.elapsed() >= Duration::from_secs(1) {
            state.published = build_snapshot(state);
            state.phases.clear();
            state.frame_count = 0;
            state.frame_wall_sum_us = 0;
            state.frame_wall_last_us = 0;
            state.period_start = Instant::now();
        }
    });
}

/// Snapshot for `systems.list` (full frame breakdown).
pub fn snapshot_for_inspector() -> Value {
    with_profiler(|state| {
        if state.frame_count > 0 || !state.phases.is_empty() {
            let live = build_snapshot(state);
            if state
                .published
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                > 0
            {
                // Merge live summary into published window for responsive UI.
                let mut merged = state.published.clone();
                if let Some(summary) = live.get("summary") {
                    merged["summary"] = summary.clone();
                }
                if let Some(render) = live.get("render") {
                    merged["render"] = render.clone();
                }
                return merged;
            }
            return live;
        }
        state.published.clone()
    })
}
