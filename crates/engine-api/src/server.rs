//! Server-only host API (simulation thread).

type SimTimeSecsFn = extern "C" fn(*mut u8) -> f32;

pub struct ServerApi {
    ctx: *mut u8,
    sim_time_secs_fn: SimTimeSecsFn,
}

impl ServerApi {
    pub fn new(ctx: *mut u8, resolver: super::Resolver) -> Self {
        let resolve = |name: &str| super::resolve_fn(resolver, name);
        Self {
            ctx,
            sim_time_secs_fn: unsafe {
                crate::fn_from_erased(resolve("v1:server:sim_time_secs"))
            },
        }
    }

    /// Monotonic server simulation time in seconds (host authority; gamelib read-only).
    pub fn sim_time_secs(&self) -> f32 {
        (self.sim_time_secs_fn)(self.ctx)
    }
}
