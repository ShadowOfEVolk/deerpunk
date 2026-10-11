use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type GetF32Fn = extern "C" fn(*mut u8, *const u8, usize, *mut f32) -> i32;
type SetFn = extern "C" fn(*mut u8, *const u8, usize, *const u8, usize) -> i32;

pub struct CvarApi {
    ctx: *mut u8,
    get_f32_fn: GetF32Fn,
    set_fn: SetFn,
}

unsafe impl Send for CvarApi {}
unsafe impl Sync for CvarApi {}

impl CvarApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            get_f32_fn: unsafe { std::mem::transmute(resolve(r, "v1:cvar:get_f32")) },
            set_fn: unsafe { std::mem::transmute(resolve(r, "v1:cvar:set")) },
        }
    }

    pub fn get_f32(&self, name: &str) -> Option<f32> {
        let bytes = name.as_bytes();
        let mut out = 0.0f32;
        let rc = (self.get_f32_fn)(self.ctx, bytes.as_ptr(), bytes.len(), &mut out);
        if rc == 0 {
            Some(out)
        } else {
            None
        }
    }

    pub fn get_f32_or(&self, name: &str, default: f32) -> f32 {
        self.get_f32(name).unwrap_or(default)
    }

    pub fn get_i32_or(&self, name: &str, default: i32) -> i32 {
        self.get_f32(name).map(|v| v as i32).unwrap_or(default)
    }

    /// Set or create a cvar (console `name value` semantics).
    pub fn set(&self, name: &str, value: &str) -> bool {
        let n = name.as_bytes();
        let v = value.as_bytes();
        (self.set_fn)(self.ctx, n.as_ptr(), n.len(), v.as_ptr(), v.len()) == 0
    }
}
