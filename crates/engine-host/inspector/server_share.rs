//! Server-world inspection snapshots published from the sim thread to the
//! main-thread inspector poll.
//!
//! Single-writer (sim thread) / single-reader (main thread) hand-off through
//! one short-critical-section [`Mutex`]; published values are shared as
//! [`Arc`] so the reader never serializes while the writer rebuilds them.
//! Direction-reversed sibling of the old `ClientInspectShare` (whose client
//! side is now read synchronously).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

struct Snapshot {
    gen: u64,
    value: Arc<Value>,
}

/// Cross-thread slot carrying server-world inspector data for the main poll.
#[derive(Default)]
pub struct ServerInspectShare {
    /// Set when an RPC asked for the entity rows / players snapshot; the sim
    /// thread rebuilds once per tick after consuming the flag, so idle ticks
    /// cost one atomic load each.
    entities_request: AtomicBool,
    players_request: AtomicBool,
    /// Single in-flight detail request: (seq, entity id).
    pending_detail: Mutex<Option<(u64, u64)>>,
    next_seq: AtomicU64,
    /// Bumped on every publish; deferred replies wait for a generation newer
    /// than their request so they never serve a pre-request snapshot.
    gen: AtomicU64,
    entities: Mutex<Option<Snapshot>>,
    players: Mutex<Option<Snapshot>>,
    /// Completed detail payloads keyed by request seq (bounded by single
    /// in-flight request).
    details: Mutex<HashMap<u64, Value>>,
}

impl ServerInspectShare {
    pub fn new() -> Self {
        Self::default()
    }

    // ── main-thread poll side ──

    pub fn request_entities(&self) {
        self.entities_request.store(true, Ordering::Relaxed);
    }

    pub fn request_players(&self) {
        self.players_request.store(true, Ordering::Relaxed);
    }

    /// Ask the sim thread for a single entity's component detail.
    pub fn request_detail(&self, entity: u64) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed) + 1;
        *self.pending_detail.lock().expect("detail queue") = Some((seq, entity));
        seq
    }

    pub(crate) fn take_entities_request(&self) -> bool {
        self.entities_request.swap(false, Ordering::Relaxed)
    }

    pub(crate) fn take_players_request(&self) -> bool {
        self.players_request.swap(false, Ordering::Relaxed)
    }

    pub(crate) fn take_pending_detail(&self) -> Option<(u64, u64)> {
        self.pending_detail.lock().expect("detail queue").take()
    }

    /// Current snapshot generation (for deferred-reply freshness checks).
    pub fn gen_now(&self) -> u64 {
        self.gen.load(Ordering::Relaxed)
    }

    pub fn entities_snapshot(&self) -> Option<(u64, Arc<Value>)> {
        self.entities
            .lock()
            .expect("entities snapshot")
            .as_ref()
            .map(|s| (s.gen, Arc::clone(&s.value)))
    }

    pub fn players_snapshot(&self) -> Option<(u64, Arc<Value>)> {
        self.players
            .lock()
            .expect("players snapshot")
            .as_ref()
            .map(|s| (s.gen, Arc::clone(&s.value)))
    }

    /// Completed detail payload for a request seq (consumed once).
    pub fn poll_detail(&self, seq: u64) -> Option<Value> {
        self.details.lock().expect("details").remove(&seq)
    }

    // ── sim-thread publisher side ──

    pub(crate) fn publish_entities(&self, value: Value) {
        let gen = self.gen.fetch_add(1, Ordering::Relaxed) + 1;
        *self.entities.lock().expect("entities snapshot") = Some(Snapshot {
            gen,
            value: Arc::new(value),
        });
    }

    pub(crate) fn publish_players(&self, value: Value) {
        let gen = self.gen.fetch_add(1, Ordering::Relaxed) + 1;
        *self.players.lock().expect("players snapshot") = Some(Snapshot {
            gen,
            value: Arc::new(value),
        });
    }

    pub(crate) fn publish_detail(&self, seq: u64, value: Value) {
        self.details.lock().expect("details").insert(seq, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_are_consumed_once() {
        let share = ServerInspectShare::new();
        assert!(!share.take_entities_request(), "idle: no request pending");
        share.request_entities();
        assert!(share.take_entities_request(), "first take after request");
        assert!(!share.take_entities_request(), "consumed");
    }

    #[test]
    fn deferred_reply_waits_for_newer_generation() {
        let share = ServerInspectShare::new();
        let wait_gen = share.gen_now();
        share.publish_entities(json!({ "count": 0 }));
        let (gen, value) = share.entities_snapshot().expect("published");
        assert!(gen > wait_gen, "publish must land after the queued request");
        assert_eq!(value.get("count").and_then(|v| v.as_i64()), Some(0));
    }

    #[test]
    fn detail_roundtrip_consumed_once() {
        let share = ServerInspectShare::new();
        let seq = share.request_detail(42);
        assert_eq!(share.take_pending_detail(), Some((seq, 42)));
        assert_eq!(share.take_pending_detail(), None, "single in-flight");
        assert!(share.poll_detail(seq).is_none(), "not answered yet");
        share.publish_detail(seq, json!({ "index": 42 }));
        assert!(share.poll_detail(seq).is_some(), "answered");
        assert!(share.poll_detail(seq).is_none(), "consumed once");
    }
}
