//! Client-world inspection data shared from the main thread to the inspector
//! (which polls on the server simulation thread).
//!
//! Single-writer (main thread) / single-reader (server thread) hand-off through
//! one short-critical-section [`Mutex`]; the row list itself is swapped as an
//! `Arc` so the reader never serializes while the writer rebuilds it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One `cliententities.list` row (mirrors the server-side row shape).
pub(crate) struct ClientEntityRow {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) prefab_id: Option<u32>,
    pub(crate) position: Option<[f32; 3]>,
}

/// Rows snapshot + the host time it was taken at.
struct RowSnapshot {
    time_secs: f32,
    rows: Arc<Vec<ClientEntityRow>>,
}

/// Cross-thread slot carrying client-world entity data for the inspector.
#[derive(Default)]
pub struct ClientInspectShare {
    /// Set by the inspector poll while WS clients are connected; the main
    /// thread only rebuilds the row snapshot when a consumer is listening.
    want_rows: AtomicBool,
    rows: Mutex<Option<RowSnapshot>>,
    /// Single in-flight detail request: (seq, entity id).
    pending: Mutex<Option<(u64, u64)>>,
    next_seq: AtomicU64,
    /// Completed detail payloads keyed by request seq (bounded).
    details: Mutex<HashMap<u64, Value>>,
}

impl ClientInspectShare {
    pub(crate) fn set_want_rows(&self, want: bool) {
        self.want_rows.store(want, Ordering::Relaxed);
    }

    pub(crate) fn want_rows(&self) -> bool {
        self.want_rows.load(Ordering::Relaxed)
    }

    /// Main thread: replace the rows snapshot.
    pub(crate) fn publish_rows(&self, time_secs: f32, rows: Vec<ClientEntityRow>) {
        let snap = RowSnapshot {
            time_secs,
            rows: Arc::new(rows),
        };
        *self.rows.lock().unwrap() = Some(snap);
    }

    /// Inspector thread: current rows snapshot (clone of the `Arc`).
    pub(crate) fn rows(&self) -> Option<(f32, Arc<Vec<ClientEntityRow>>)> {
        let guard = self.rows.lock().unwrap();
        guard.as_ref().map(|s| (s.time_secs, Arc::clone(&s.rows)))
    }

    /// Inspector thread: enqueue a component-detail request; returns its seq.
    pub(crate) fn request_detail(&self, entity: u64) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        *self.pending.lock().unwrap() = Some((seq, entity));
        seq
    }

    /// Main thread: take the queued detail request, if any.
    pub(crate) fn take_pending(&self) -> Option<(u64, u64)> {
        self.pending.lock().unwrap().take()
    }

    /// Main thread: deliver a computed detail payload (or `{"_error": ...}`).
    pub(crate) fn submit_detail(&self, seq: u64, value: Value) {
        let mut details = self.details.lock().unwrap();
        // Keep the map bounded: drop anything older than the last few seqs.
        details.retain(|s, _| seq.saturating_sub(*s) < 64);
        details.insert(seq, value);
    }

    /// Inspector thread: fetch a completed detail payload, consuming it.
    pub(crate) fn poll_detail(&self, seq: u64) -> Option<Value> {
        self.details.lock().unwrap().remove(&seq)
    }
}
