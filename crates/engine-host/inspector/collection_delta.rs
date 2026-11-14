//! Delta bookkeeping for one inspector collection: a monotonic revision plus
//! a ring of the last changed snapshots, diffed on `{col}.list` `since`.
//!
//! Lives entirely on the thread that builds the snapshots (sim thread for
//! `serverentities`/`players`, main thread for `cliententities`), so no
//! per-revision state crosses threads.

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Value};

/// How many changed snapshots stay diffable. A `since` older than the oldest
/// ring entry (e.g. a reconnecting studio) degrades to a full snapshot reply.
const RING_CAP: usize = 8;

/// Reply payload for a `{col}.list` request with `since`.
pub(crate) enum ListReply {
    /// Content unchanged since `since`: empty delta at the current revision.
    Unchanged(Value),
    /// Delta against the ring snapshot with revision `since`.
    Delta(Value),
    /// Unknown / stale `since` (ring miss, revision 0): full snapshot.
    Full(Value),
}

/// Revision + snapshot ring for one collection.
pub(crate) struct CollectionDelta {
    revision: u64,
    ring: VecDeque<(u64, HashMap<u64, Value>)>,
}

/// Row id → row for a snapshot body (`rows_key` is `"entities"` / `"players"`).
fn rows_of<'a>(value: &'a Value, rows_key: &str) -> HashMap<u64, &'a Value> {
    value
        .get(rows_key)
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = row.get("index").and_then(Value::as_u64)?;
                    Some((id, row))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl CollectionDelta {
    pub(crate) fn new() -> Self {
        Self {
            revision: 0,
            ring: VecDeque::new(),
        }
    }

    /// Current collection revision (monotonic; bumped only when content
    /// actually changes).
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Record a fresh snapshot; returns the current revision. A
    /// content-identical snapshot keeps the revision and adds no ring entry,
    /// so idle polls stay free.
    pub(crate) fn publish(&mut self, fresh: &Value, rows_key: &str) -> u64 {
        let rows = rows_of(fresh, rows_key);
        if let Some((_, prev)) = self.ring.back() {
            if rows_equal(prev, &rows) {
                return self.revision;
            }
        }
        self.revision += 1;
        let owned: HashMap<u64, Value> = rows.into_iter().map(|(id, r)| (id, r.clone())).collect();
        self.ring.push_back((self.revision, owned));
        while self.ring.len() > RING_CAP {
            self.ring.pop_front();
        }
        self.revision
    }

    /// Build the reply for a `{col}.list` request carrying `since` against
    /// the fresh snapshot `fresh` (already published via [`Self::publish`]).
    pub(crate) fn list_reply(&self, since: u64, fresh: &Value, rows_key: &str) -> ListReply {
        if since == 0 {
            return ListReply::Full(full_reply(self.revision, fresh));
        }
        if since == self.revision {
            return ListReply::Unchanged(unchanged_reply(self.revision));
        }
        if since > self.revision {
            // Client ahead of us (e.g. server restarted): resync fully.
            return ListReply::Full(full_reply(self.revision, fresh));
        }
        let Some((_, base)) = self.ring.iter().find(|(rev, _)| *rev == since) else {
            return ListReply::Full(full_reply(self.revision, fresh));
        };
        let fresh_rows = rows_of(fresh, rows_key);
        let mut added = Vec::new();
        let mut updated = Vec::new();
        for (id, row) in &fresh_rows {
            match base.get(id) {
                None => added.push((*row).clone()),
                Some(prev) if *prev != **row => updated.push((*row).clone()),
                _ => {}
            }
        }
        let removed: Vec<Value> = base
            .keys()
            .filter(|id| !fresh_rows.contains_key(id))
            .map(|id| json!(id))
            .collect();
        ListReply::Delta(json!({
            "revision": self.revision,
            "full": false,
            "added": added,
            "updated": updated,
            "removed": removed,
        }))
    }
}

fn rows_equal(prev: &HashMap<u64, Value>, fresh: &HashMap<u64, &Value>) -> bool {
    prev.len() == fresh.len()
        && prev
            .iter()
            .all(|(id, row)| fresh.get(id).copied() == Some(row))
}

fn full_reply(revision: u64, fresh: &Value) -> Value {
    let mut reply = fresh.clone();
    if let Some(obj) = reply.as_object_mut() {
        obj.insert("revision".into(), json!(revision));
        obj.insert("full".into(), json!(true));
    }
    reply
}

fn unchanged_reply(revision: u64) -> Value {
    json!({
        "revision": revision,
        "full": false,
        "added": [],
        "updated": [],
        "removed": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(rows: &[Value]) -> Value {
        json!({ "time": 1.0, "count": rows.len(), "entities": rows })
    }

    fn row(id: u64, rev: u64) -> Value {
        json!({ "index": id, "rev": rev })
    }

    #[test]
    fn unchanged_snapshot_keeps_revision() {
        let mut delta = CollectionDelta::new();
        let a = snapshot(&[row(1, 1), row(2, 1)]);
        assert_eq!(delta.publish(&a, "entities"), 1);
        assert_eq!(
            delta.publish(&a, "entities"),
            1,
            "identical content: no bump"
        );
        // Row order change alone is not a content change.
        let reordered = snapshot(&[row(2, 1), row(1, 1)]);
        assert_eq!(delta.publish(&reordered, "entities"), 1);
    }

    #[test]
    fn diff_reports_added_updated_removed() {
        let mut delta = CollectionDelta::new();
        delta.publish(&snapshot(&[row(1, 1), row(2, 1), row(3, 1)]), "entities");
        let since = delta.revision();

        let fresh = snapshot(&[row(1, 2), row(3, 1), row(4, 1)]);
        let rev = delta.publish(&fresh, "entities");
        assert_eq!(rev, 2);
        match delta.list_reply(since, &fresh, "entities") {
            ListReply::Delta(reply) => {
                assert_eq!(reply["revision"], rev);
                assert_eq!(reply["full"], false);
                assert_eq!(reply["added"], json!([row(4, 1)]));
                assert_eq!(reply["updated"], json!([row(1, 2)]));
                assert_eq!(reply["removed"], json!([2]));
            }
            _ => panic!("expected delta reply"),
        }
    }

    #[test]
    fn since_at_current_revision_is_empty_delta() {
        let mut delta = CollectionDelta::new();
        let a = snapshot(&[row(1, 1)]);
        let rev = delta.publish(&a, "entities");
        match delta.list_reply(rev, &a, "entities") {
            ListReply::Unchanged(reply) => {
                assert_eq!(reply["revision"], rev);
                assert_eq!(reply["added"], json!([]));
                assert_eq!(reply["removed"], json!([]));
            }
            _ => panic!("expected unchanged reply"),
        }
    }

    #[test]
    fn stale_since_falls_back_to_full() {
        let mut delta = CollectionDelta::new();
        // Push RING_CAP + 2 revisions; revision 1 scrolls out of the ring.
        let mut last = snapshot(&[]);
        for i in 0..(RING_CAP + 2) {
            last = snapshot(&[row(1, i as u64)]);
            delta.publish(&last, "entities");
        }
        match delta.list_reply(1, &last, "entities") {
            ListReply::Full(reply) => {
                assert_eq!(reply["revision"], delta.revision());
                assert_eq!(reply["full"], true);
                assert_eq!(reply["count"], 1);
            }
            _ => panic!("expected full reply"),
        }
    }

    #[test]
    fn zero_since_is_full_reply() {
        let mut delta = CollectionDelta::new();
        let a = snapshot(&[row(1, 1)]);
        delta.publish(&a, "entities");
        match delta.list_reply(0, &a, "entities") {
            ListReply::Full(reply) => {
                assert_eq!(reply["revision"], 1);
                assert_eq!(reply["full"], true);
                assert_eq!(reply["entities"], json!([row(1, 1)]));
            }
            _ => panic!("expected full reply"),
        }
    }

    #[test]
    fn since_newer_than_revision_resyncs_full() {
        let mut delta = CollectionDelta::new();
        let a = snapshot(&[]);
        delta.publish(&a, "entities");
        match delta.list_reply(999, &a, "entities") {
            ListReply::Full(reply) => assert_eq!(reply["full"], true),
            _ => panic!("expected full reply"),
        }
    }
}
