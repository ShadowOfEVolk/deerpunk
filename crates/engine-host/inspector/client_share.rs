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
    /// Set when an RPC asked for the row list; the main thread rebuilds the
    /// row snapshot only after consuming this flag, so idle frames cost one
    /// atomic load.
    rows_request: AtomicBool,
    rows: Mutex<Option<RowSnapshot>>,
    /// Bumped on every `publish_rows`; lets deferred list replies wait for a
    /// snapshot taken AFTER their request instead of serving a stale one.
    rows_gen: AtomicU64,
    /// Single in-flight detail request: (seq, entity id).
    pending: Mutex<Option<(u64, u64)>>,
    next_seq: AtomicU64,
    /// Completed detail payloads keyed by request seq (bounded).
    details: Mutex<HashMap<u64, Value>>,
}

impl ClientInspectShare {
    /// Inspector thread: an RPC asked for the row list; the main thread
    /// rebuilds the snapshot once on its next pump.
    pub(crate) fn request_rows(&self) {
        self.rows_request.store(true, Ordering::Relaxed);
    }

    /// Main thread: consume the rows request; `true` exactly once per RPC ask.
    pub(crate) fn take_rows_request(&self) -> bool {
        self.rows_request.swap(false, Ordering::Relaxed)
    }

    /// Main thread: replace the rows snapshot.
    pub(crate) fn publish_rows(&self, time_secs: f32, rows: Vec<ClientEntityRow>) {
        let snap = RowSnapshot {
            time_secs,
            rows: Arc::new(rows),
        };
        *self.rows.lock().unwrap() = Some(snap);
        self.rows_gen.fetch_add(1, Ordering::Relaxed);
    }

    /// Inspector thread: current rows snapshot (clone of the `Arc`).
    pub(crate) fn rows(&self) -> Option<(f32, Arc<Vec<ClientEntityRow>>)> {
        let guard = self.rows.lock().unwrap();
        guard.as_ref().map(|s| (s.time_secs, Arc::clone(&s.rows)))
    }

    /// Inspector thread: generation of the current snapshot (0 before the
    /// first publish). A deferred list queued at gen N must wait for gen > N.
    pub(crate) fn rows_gen(&self) -> u64 {
        self.rows_gen.load(Ordering::Relaxed)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_request_is_consumed_once() {
        let share = ClientInspectShare::default();
        assert!(!share.take_rows_request(), "idle: no request pending");
        share.request_rows();
        assert!(share.take_rows_request(), "first take after request");
        assert!(!share.take_rows_request(), "flag is swap-and-clear");
        share.request_rows();
        share.request_rows();
        assert!(share.take_rows_request());
        assert!(!share.take_rows_request(), "coalesced asks need one rebuild");
    }

    #[test]
    fn detail_roundtrip_and_not_found_shape() {
        let share = ClientInspectShare::default();
        let seq = share.request_detail(42);
        assert_eq!(share.take_pending(), Some((seq, 42)));
        assert_eq!(share.take_pending(), None, "single in-flight request");
        share.submit_detail(seq, json!({ "_error": "entity not found" }));
        assert_eq!(share.poll_detail(seq), Some(json!({ "_error": "entity not found" })));
        assert_eq!(share.poll_detail(seq), None, "payload consumed once");
    }

    #[test]
    fn rows_gen_advances_only_on_publish() {
        let share = ClientInspectShare::default();
        assert_eq!(share.rows_gen(), 0, "no snapshot published yet");
        let queued_at = share.rows_gen();
        // A publish must land before a deferred reply queued earlier is fresh.
        assert!(share.rows_gen() <= queued_at);
        share.publish_rows(1.0, Vec::new());
        assert!(share.rows_gen() > queued_at, "deferred reply unblocks");
        let queued_later = share.rows_gen();
        assert!(share.rows_gen() <= queued_later, "stale snapshot stays stale");
        share.publish_rows(2.0, Vec::new());
        assert!(share.rows_gen() > queued_later);
    }
}
