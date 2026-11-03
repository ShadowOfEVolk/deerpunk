//! Thin host wrapper for [`deerpunk_engine_common::world::World::register_progress_system`].

use deerpunk_engine_common::world::{ProgressSystemFn, World};

pub use deerpunk_engine_common::world::ProgressSystemFn as WorldProgressSystemFn;

pub fn register_progress_system(
    world: &World,
    name: &str,
    query: &str,
    run: ProgressSystemFn,
    user_data: *mut std::ffi::c_void,
    interval_secs: f32,
) -> u64 {
    world.register_progress_system(name, query, run, user_data, interval_secs)
}

pub fn set_progress_system_interval(world: &World, system_id: u64, interval_secs: f32) {
    world.set_progress_system_interval(system_id, interval_secs);
}
