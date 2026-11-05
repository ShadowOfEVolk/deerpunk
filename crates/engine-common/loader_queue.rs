//! Gamelib-visible load progress (startup / map / batched resource loads).

use std::sync::Mutex;

/// Snapshot for UI or inspector (`v1:host:loader_queue_*`).
#[derive(Clone, Debug, Default)]
pub struct LoaderQueueSnapshot {
    pub active: bool,
    pub done: u32,
    pub total: u32,
    pub status: String,
    pub hint: String,
}

#[derive(Default)]
pub struct LoaderQueue {
    inner: Mutex<LoaderQueueSnapshot>,
}

impl LoaderQueue {
    pub fn set(
        &self,
        active: bool,
        done: u32,
        total: u32,
        status: impl Into<String>,
        hint: impl Into<String>,
    ) {
        let total = total.max(1);
        let mut g = self.inner.lock().expect("loader_queue");
        g.active = active;
        g.done = done.min(total);
        g.total = total;
        g.status = status.into();
        g.hint = hint.into();
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().expect("loader_queue");
        *g = LoaderQueueSnapshot::default();
    }

    pub fn snapshot(&self) -> LoaderQueueSnapshot {
        self.inner.lock().expect("loader_queue").clone()
    }
}
