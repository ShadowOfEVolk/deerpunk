use std::ffi::c_void;

use crate::Resolver;

pub type ExternSystemFn = extern "C" fn(delta_time: f32, user_data: *mut c_void);

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct ScheduleApi {
    ctx: *mut u8,
    add_system_fn: extern "C" fn(*mut u8, ExternSystemFn, *mut c_void),
    clear_fn: extern "C" fn(*mut u8),
    run_fn: extern "C" fn(*mut u8, f32),
}

unsafe impl Send for ScheduleApi {}
unsafe impl Sync for ScheduleApi {}

impl ScheduleApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            add_system_fn: unsafe { std::mem::transmute(resolve(r, "v1:schedule:add_system")) },
            clear_fn: unsafe { std::mem::transmute(resolve(r, "v1:schedule:clear")) },
            run_fn: unsafe { std::mem::transmute(resolve(r, "v1:schedule:run")) },
        }
    }

    pub fn add_system_raw(&self, system_fn: ExternSystemFn, user_data: *mut c_void) {
        (self.add_system_fn)(self.ctx, system_fn, user_data);
    }

    pub fn clear(&self) {
        (self.clear_fn)(self.ctx);
    }

    pub fn run(&self, delta_time: f32) {
        (self.run_fn)(self.ctx, delta_time);
    }
}
