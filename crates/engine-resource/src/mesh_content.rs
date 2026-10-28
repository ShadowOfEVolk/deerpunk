//! Monotonic mesh content ids — unique for the process lifetime (never reused).

use std::sync::atomic::{AtomicU64, Ordering};

/// Invalid / unset mesh content id.
pub const MESH_CONTENT_INVALID: u64 = 0;

static NEXT_MESH_CONTENT_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a new mesh content id (thread-safe, never returns 0).
pub fn allocate_mesh_content_id() -> u64 {
    let id = NEXT_MESH_CONTENT_ID.fetch_add(1, Ordering::Relaxed);
    assert_ne!(id, MESH_CONTENT_INVALID);
    id
}
