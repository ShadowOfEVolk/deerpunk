use crate::Resolver;

fn try_resolve(r: Resolver, name: &str) -> Option<*const ()> {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

type ZoneBeginFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;
type ZoneEndFn = extern "C" fn(*mut u8) -> i32;
type FrameMarkFn = extern "C" fn(*mut u8) -> i32;

pub struct ProfileApi {
    ctx: *mut u8,
    zone_begin: Option<ZoneBeginFn>,
    zone_end: Option<ZoneEndFn>,
    frame_mark: Option<FrameMarkFn>,
}

unsafe impl Send for ProfileApi {}
unsafe impl Sync for ProfileApi {}

impl ProfileApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        let zone_begin = try_resolve(r, "v1:profile:zone_begin").map(|p| unsafe {
            std::mem::transmute(p)
        });
        let zone_end = try_resolve(r, "v1:profile:zone_end").map(|p| unsafe {
            std::mem::transmute(p)
        });
        let frame_mark = try_resolve(r, "v1:profile:frame_mark").map(|p| unsafe {
            std::mem::transmute(p)
        });
        Self {
            ctx,
            zone_begin,
            zone_end,
            frame_mark,
        }
    }

    pub fn host_zones_available(&self) -> bool {
        self.zone_begin.is_some() && self.zone_end.is_some()
    }

    pub fn zone_begin(&self, name: &str) {
        if let Some(f) = self.zone_begin {
            let _ = (f)(self.ctx, name.as_ptr(), name.len());
        }
    }

    pub fn zone_end(&self) {
        if let Some(f) = self.zone_end {
            let _ = (f)(self.ctx);
        }
    }

    pub fn frame_mark(&self) {
        if let Some(f) = self.frame_mark {
            let _ = (f)(self.ctx);
        }
    }
}
