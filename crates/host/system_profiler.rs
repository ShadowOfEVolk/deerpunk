//! Per-phase host timings for the inspector `systems` collection (darkdemo-style).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

struct ProfilerState {
    /// Accumulator for the current 1s window.
    sums: HashMap<&'static str, u64>,
    frame_count: u16,
    frame_time_sum_us: u64,
    period_start: Instant,
    /// Last published window (what `systems.list` returns).
    published: Value,
}

impl ProfilerState {
    fn new() -> Self {
        Self {
            sums: HashMap::new(),
            frame_count: 0,
            frame_time_sum_us: 0,
            period_start: Instant::now(),
            published: json!({
                "frame_count": 0,
                "period_secs": 0.0,
                "systems": [],
            }),
        }
    }
}

static PROFILER: OnceLock<Mutex<ProfilerState>> = OnceLock::new();

thread_local! {
    static FRAME_START: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

fn with_profiler<R>(f: impl FnOnce(&mut ProfilerState) -> R) -> R {
    let mut g = PROFILER
        .get_or_init(|| Mutex::new(ProfilerState::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f(&mut g)
}

/// Mark start of a host frame (wall-clock).
pub fn begin_frame() {
    FRAME_START.set(Some(Instant::now()));
}

/// Record one named phase duration for the current frame.
pub fn record(name: &'static str, elapsed: Duration) {
    let micros = elapsed.as_micros().min(u128::from(u32::MAX)) as u64;
    with_profiler(|state| {
        *state.sums.entry(name).or_insert(0) += micros;
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

fn build_snapshot(state: &ProfilerState) -> Value {
    let total_us: u64 = state.sums.values().copied().sum();
    let frames = u64::from(state.frame_count.max(1));
    let period_secs = state.period_start.elapsed().as_secs_f32();

    let mut systems: Vec<Value> = state
        .sums
        .iter()
        .map(|(name, time_us)| {
            let avg_us = time_us / frames;
            let percent = if total_us > 0 {
                (*time_us as f64 / total_us as f64) * 100.0
            } else {
                0.0
            };
            let avg_ms = (avg_us as f64) / 1000.0;
            json!({
                "name": name,
                "time_ms_avg": (avg_ms * 100.0).round() / 100.0,
                "time_us_avg": avg_us,
                "percent": (percent * 10.0).round() / 10.0,
            })
        })
        .collect();

    systems.sort_by(|a, b| {
        b.get("time_ms_avg")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .partial_cmp(&a.get("time_ms_avg").and_then(|v| v.as_f64()).unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    json!({
        "frame_count": state.frame_count,
        "period_secs": period_secs,
        "systems": systems,
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
        state.frame_time_sum_us += frame_us;

        if state.period_start.elapsed() >= Duration::from_secs(1) {
            state.published = build_snapshot(state);
            state.sums.clear();
            state.frame_count = 0;
            state.frame_time_sum_us = 0;
            state.period_start = Instant::now();
        }
    });
}

/// Snapshot for `systems.list` (same shape as classic darkdemo inspector).
pub fn snapshot_for_inspector() -> Value {
    with_profiler(|state| {
        // Live window if nothing published yet.
        if state
            .published
            .get("systems")
            .and_then(|v| v.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true)
            && !state.sums.is_empty()
        {
            return build_snapshot(state);
        }
        state.published.clone()
    })
}
