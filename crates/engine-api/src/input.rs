use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

// USB HID Scancode constants (cross-platform, layout-independent)
pub const KEY_A: u16 = 4;
pub const KEY_D: u16 = 7;
pub const KEY_E: u16 = 8;
pub const KEY_S: u16 = 22;
pub const KEY_W: u16 = 26;
pub const KEY_SPACE: u16 = 44;
pub const KEY_TAB: u16 = 43;
pub const KEY_ESCAPE: u16 = 41;
pub const KEY_RIGHT: u16 = 79;
pub const KEY_LEFT: u16 = 80;
pub const KEY_DOWN: u16 = 81;
pub const KEY_UP: u16 = 82;
pub const KEY_F1: u16 = 58;
pub const KEY_F2: u16 = 59;
pub const KEY_F3: u16 = 60;
pub const KEY_F4: u16 = 61;
pub const KEY_LCTRL: u16 = 224;
pub const KEY_LSHIFT: u16 = 225;

pub struct InputApi {
    ctx: *mut u8,
    is_key_down_fn: extern "C" fn(*mut u8, u32) -> bool,
    mouse_dx_fn: extern "C" fn(*mut u8) -> f32,
    mouse_dy_fn: extern "C" fn(*mut u8) -> f32,
    set_relative_mouse_fn: extern "C" fn(*mut u8, i32) -> i32,
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
            set_relative_mouse_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:input:set_relative_mouse"))
            },
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

    pub fn set_relative_mouse(&self, on: bool) {
        (self.set_relative_mouse_fn)(self.ctx, if on { 1 } else { 0 });
    }
}
