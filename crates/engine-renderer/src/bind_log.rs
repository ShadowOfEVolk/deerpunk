//! [`r_logbindgroups`] — one line per `set_bind_group` in the mesh pass.
//! `repeat` means this slot was bound to the same description earlier in the pass.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

const SLOTS: usize = 4;

std::thread_local! {
    static LAST: [Cell<u64>; SLOTS] = [const { Cell::new(0) }; SLOTS];
    static COUNTS: [Cell<u32>; SLOTS] = [const { Cell::new(0) }; SLOTS];
    static REPEATS: [Cell<u32>; SLOTS] = [const { Cell::new(0) }; SLOTS];
}

/// Synced from [`r_logbindgroups`] each frame via client `apply_renderer_cvars`.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn begin_pass() {
    if !enabled() {
        return;
    }
    LAST.with(|slots| {
        for slot in slots {
            slot.set(0);
        }
    });
    COUNTS.with(|slots| {
        for slot in slots {
            slot.set(0);
        }
    });
    REPEATS.with(|slots| {
        for slot in slots {
            slot.set(0);
        }
    });
}

pub fn group(slot: u32, detail: &str) {
    if !enabled() || slot as usize >= SLOTS {
        return;
    }
    let hash = fnv(detail);
    let i = slot as usize;
    let same = LAST.with(|slots| {
        let prev = slots[i].get();
        slots[i].set(hash);
        prev == hash && prev != 0
    });
    COUNTS.with(|c| c[i].set(c[i].get() + 1));
    if same {
        REPEATS.with(|c| c[i].set(c[i].get() + 1));
    }
    log::info!(
        "[bind] g{slot} {} {detail}",
        if same { "repeat" } else { "new" }
    );
}

pub fn end_pass() {
    if !enabled() {
        return;
    }
    let mut parts = String::new();
    for i in 0..SLOTS {
        let n = COUNTS.with(|c| c[i].get());
        if n == 0 {
            continue;
        }
        let rep = REPEATS.with(|c| c[i].get());
        parts.push_str(&format!(" g{i}={n}(repeat {rep})"));
    }
    log::info!("[bind] mesh pass{parts}");
}

fn fnv(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    if h == 0 {
        1
    } else {
        h
    }
}
