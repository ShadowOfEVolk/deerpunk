//! Register Flecs observers on a [`World`] (client presentation hooks from gamelib).

use std::ffi::{c_void, CString};

use flecs_ecs_sys as sys;

use crate::world::World;

struct ObserverSlot {
    _query: CString,
    callback: extern "C" fn(u64),
}

/// Gamelib callback: one entity per matching observer event.
pub type ClientObserverFn = extern "C" fn(u64);

unsafe extern "C-unwind" fn observer_bridge(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let slot = &*(it_ref.ctx as *const ObserverSlot);
        for i in 0..it_ref.count as usize {
            let e = *it_ref.entities.add(i);
            (slot.callback)(e);
        }
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
    let query_c = CString::new(query).map_err(|e| e.to_string())?;
    let slot = Box::leak(Box::new(ObserverSlot {
        _query: query_c,
        callback,
    }));
    let query_ptr = slot._query.as_ptr();
    unsafe {
        let mut desc: sys::ecs_observer_desc_t = std::mem::zeroed();
        desc.query.expr = query_ptr;
        desc.callback = Some(observer_bridge);
        desc.ctx = slot as *mut ObserverSlot as *mut c_void;
        let mut i = 0usize;
        if on_add {
            desc.events[i] = sys::EcsOnAdd;
            i += 1;
        }
        if on_set {
            desc.events[i] = sys::EcsOnSet;
        }
        let ent = sys::ecs_observer_init(world.as_ptr(), &desc);
        if ent == 0 {
            return Err(format!("ecs_observer_init failed for query '{query}'"));
        }
    }
    log::info!(target: "ecs", "observer '{query}' add={on_add} set={on_set}");
    Ok(())
}
