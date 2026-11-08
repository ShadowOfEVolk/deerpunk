//! Client presentation observers. Flecs stays in `deerpunk-engine-world`.

use crate::world::{EcsCallback, EntityId, WatchEvent, World};

pub type ClientObserverFn = extern "C" fn(u64);

struct ObserverCtx {
    callback: ClientObserverFn,
}

extern "C-unwind" fn observer_bridge(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let slot = unsafe { &*(ctx as *const ObserverCtx) };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        (slot.callback)(e);
    }
}

/// `query` is a Flecs query expression (e.g. `ItemDropComponent`).
pub fn register_client_observer(
    world: &World,
    query: &str,
    on_add: bool,
    on_set: bool,
    callback: ClientObserverFn,
) -> Result<(), String> {
    if !on_add && !on_set {
        return Err("register_client_observer: no events".into());
    }
    let mut events = Vec::new();
    if on_add {
        events.push(WatchEvent::OnAdd);
    }
    if on_set {
        events.push(WatchEvent::OnSet);
    }
    let ctx = Box::leak(Box::new(ObserverCtx { callback }));
    world.register_observer(
        query,
        &events,
        ctx as *mut ObserverCtx as *mut std::ffi::c_void,
        observer_bridge as EcsCallback,
    )
}
