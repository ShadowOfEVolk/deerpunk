//! Tracy zones for gamelib DLLs (single profiler in the host executable).

#[cfg(feature = "tracy")]
mod imp {
    use std::cell::RefCell;

    use crate::tracy_lifecycle::tracy_ensure_started;

    thread_local! {
        static ZONE_STACK: RefCell<Vec<tracy_client::Span>> = RefCell::new(Vec::new());
    }

    pub fn zone_begin(name: &str) {
        tracy_ensure_started();
        let Some(client) = tracy_client::Client::running() else {
            return;
        };
        let span = client.span_alloc(Some(name), "gamelib", "ohmyblock", 0, 0);
        ZONE_STACK.with(|s| s.borrow_mut().push(span));
    }

    pub fn zone_end() {
        ZONE_STACK.with(|s| {
            s.borrow_mut().pop();
        });
    }

    pub fn frame_mark() {
        if tracy_client::Client::is_running() {
            tracy_client::frame_mark();
        }
    }
}

#[cfg(not(feature = "tracy"))]
mod imp {
    pub fn zone_begin(_name: &str) {}
    pub fn zone_end() {}
    pub fn frame_mark() {}
}

pub use imp::{frame_mark, zone_begin, zone_end};
