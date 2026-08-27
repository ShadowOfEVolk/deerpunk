use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct ImGuiApi {
    ctx: *mut u8,
    begin_window_fn: extern "C" fn(*mut u8, *const u8, usize) -> bool,
    end_window_fn: extern "C" fn(*mut u8),
    text_fn: extern "C" fn(*mut u8, *const u8, usize),
    value_f32_fn: extern "C" fn(*mut u8, *const u8, usize, f32),
    value_vec3_fn: extern "C" fn(*mut u8, *const u8, usize, f32, f32, f32),
    separator_fn: extern "C" fn(*mut u8),
}

unsafe impl Send for ImGuiApi {}
unsafe impl Sync for ImGuiApi {}

impl ImGuiApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            begin_window_fn: unsafe { std::mem::transmute(resolve(r, "v1:imgui:begin_window")) },
            end_window_fn: unsafe { std::mem::transmute(resolve(r, "v1:imgui:end_window")) },
            text_fn: unsafe { std::mem::transmute(resolve(r, "v1:imgui:text")) },
            value_f32_fn: unsafe { std::mem::transmute(resolve(r, "v1:imgui:value_f32")) },
            value_vec3_fn: unsafe { std::mem::transmute(resolve(r, "v1:imgui:value_vec3")) },
            separator_fn: unsafe { std::mem::transmute(resolve(r, "v1:imgui:separator")) },
        }
    }

    pub fn begin_window(&self, name: &str) -> bool {
        let bytes = name.as_bytes();
        (self.begin_window_fn)(self.ctx, bytes.as_ptr(), bytes.len())
    }

    pub fn end_window(&self) {
        (self.end_window_fn)(self.ctx);
    }

    pub fn text(&self, text: &str) {
        let bytes = text.as_bytes();
        (self.text_fn)(self.ctx, bytes.as_ptr(), bytes.len());
    }

    pub fn value_f32(&self, label: &str, value: f32) {
        let bytes = label.as_bytes();
        (self.value_f32_fn)(self.ctx, bytes.as_ptr(), bytes.len(), value);
    }

    pub fn value_vec3(&self, label: &str, x: f32, y: f32, z: f32) {
        let bytes = label.as_bytes();
        (self.value_vec3_fn)(self.ctx, bytes.as_ptr(), bytes.len(), x, y, z);
    }

    pub fn separator(&self) {
        (self.separator_fn)(self.ctx);
    }
}
