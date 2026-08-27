use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct InputApi {
    ctx: *mut u8,
    is_key_down_fn: extern "C" fn(*mut u8, u32) -> bool,
    mouse_dx_fn: extern "C" fn(*mut u8) -> f32,
    mouse_dy_fn: extern "C" fn(*mut u8) -> f32,
}

unsafe impl Send for InputApi {}
unsafe impl Sync for InputApi {}

impl InputApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            is_key_down_fn: unsafe { std::mem::transmute(resolve(r, "v1:input:is_key_down")) },
            mouse_dx_fn: unsafe { std::mem::transmute(resolve(r, "v1:input:mouse_dx")) },
            mouse_dy_fn: unsafe { std::mem::transmute(resolve(r, "v1:input:mouse_dy")) },
        }
    }

    pub fn is_key_down(&self, key: u32) -> bool {
        (self.is_key_down_fn)(self.ctx, key)
    }

    pub fn mouse_dx(&self) -> f32 {
        (self.mouse_dx_fn)(self.ctx)
    }

    pub fn mouse_dy(&self) -> f32 {
        (self.mouse_dy_fn)(self.ctx)
    }
}
