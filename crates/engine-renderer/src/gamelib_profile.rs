//! Profiler scopes for gamelib DLLs (single collector in the host executable).

#[cfg(feature = "tracy")]
mod tracy_imp {
    use std::cell::RefCell;

    use crate::tracy_lifecycle::tracy_ensure_started;

    const MAX_ZONE_LABEL: usize = 240;

    thread_local! {
        static ZONE_STACK: RefCell<Vec<ZoneEntry>> = RefCell::new(Vec::new());
    }

    enum ZoneEntry {
        Active(tracy_client::Span),
        Skipped,
    }

    fn truncate_label(name: &str) -> &str {
        if name.len() <= MAX_ZONE_LABEL {
            return name;
        }
        let mut end = MAX_ZONE_LABEL;
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        &name[..end]
    }

    pub fn zone_begin(name: &str) {
        tracy_ensure_started();
        let label = truncate_label(name);
        let entry = if let Some(client) = tracy_client::Client::running() {
            let span = client.span_alloc(Some(label), "gamelib", "zone", 0, 0);
            ZoneEntry::Active(span)
        } else {
            ZoneEntry::Skipped
        };
        ZONE_STACK.with(|s| s.borrow_mut().push(entry));
    }

    pub fn zone_end() {
        ZONE_STACK.with(|s| {
            match s.borrow_mut().pop() {
                Some(ZoneEntry::Active(span)) => drop(span),
                Some(ZoneEntry::Skipped) => {}
                None => {}
            }
        });
    }

    pub fn frame_mark() {
        if tracy_client::Client::is_running() {
            tracy_client::frame_mark();
        }
    }

    pub struct ZoneScope;

    impl Drop for ZoneScope {
        fn drop(&mut self) {
            zone_end();
        }
    }

    pub fn zone_scope(name: &str) -> ZoneScope {
        zone_begin(name);
        ZoneScope
    }

    pub fn message(text: &str) {
        tracy_ensure_started();
        let Some(client) = tracy_client::Client::running() else {
            return;
        };
        const MAX_MSG: usize = 16_000;
        let msg = if text.len() <= MAX_MSG {
            text
        } else {
            let mut end = MAX_MSG;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        };
        client.message(msg, 0);
    }
}

#[cfg(feature = "puffin")]
mod puffin_imp {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};

    use puffin::{ProfilerScope, ScopeId};

    use crate::puffin_lifecycle::puffin_ensure_started;

    static NAMED_IDS: LazyLock<Mutex<HashMap<String, ScopeId>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    thread_local! {
        static ZONE_STACK: RefCell<Vec<Option<ProfilerScope>>> = RefCell::new(Vec::new());
    }

    fn scope_id(name: &str) -> ScopeId {
        let mut ids = NAMED_IDS.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(&id) = ids.get(name) {
            return id;
        }
        let id = puffin::ThreadProfiler::call(|tp| {
            tp.register_named_scope(name.to_string(), "gamelib", "zone", 0)
        });
        ids.insert(name.to_string(), id);
        id
    }

    pub fn zone_begin(name: &str) {
        puffin_ensure_started();
        let scope = if puffin::are_scopes_on() {
            Some(ProfilerScope::new(scope_id(name), name))
        } else {
            None
        };
        ZONE_STACK.with(|s| s.borrow_mut().push(scope));
    }

    pub fn zone_end() {
        ZONE_STACK.with(|s| {
            s.borrow_mut().pop();
        });
    }

    pub fn frame_mark() {}

    pub struct ZoneScope;

    impl Drop for ZoneScope {
        fn drop(&mut self) {
            zone_end();
        }
    }

    pub fn zone_scope(name: &str) -> ZoneScope {
        zone_begin(name);
        ZoneScope
    }

    pub fn message(_text: &str) {}
}

#[cfg(not(any(feature = "tracy", feature = "puffin")))]
mod noop_imp {
    pub struct ZoneScope;

    pub fn zone_begin(_name: &str) {}
    pub fn zone_end() {}
    pub fn zone_scope(_name: &str) -> ZoneScope {
        ZoneScope
    }
    pub fn frame_mark() {}
    pub fn message(_text: &str) {}
}

#[cfg(feature = "tracy")]
pub use tracy_imp::*;

#[cfg(feature = "puffin")]
pub use puffin_imp::*;

#[cfg(not(any(feature = "tracy", feature = "puffin")))]
pub use noop_imp::*;

#[cfg(any(feature = "tracy", feature = "puffin"))]
pub use self::ZoneScope;
