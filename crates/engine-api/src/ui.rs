use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type StrFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;

/// Client UI widget list API (`v1:ui:*`).
pub struct UiApi {
    ctx: *mut u8,
    load_fn: StrFn,
    unload_fn: StrFn,
    show_fn: StrFn,
    hide_fn: StrFn,
}

unsafe impl Send for UiApi {}
unsafe impl Sync for UiApi {}

impl UiApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            load_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:load")) },
            unload_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:unload")) },
            show_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:show")) },
            hide_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:hide")) },
        }
    }

    fn call(f: StrFn, ctx: *mut u8, id: &str) -> Result<(), String> {
        let bytes = id.as_bytes();
        let rc = f(ctx, bytes.as_ptr(), bytes.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("ui op failed: {id}"))
        }
    }

    /// Load document into the widget list (hidden). Id: `main_menu` or `widgets/main_menu.rml`.
    pub fn load(&self, id: &str) -> Result<(), String> {
        Self::call(self.load_fn, self.ctx, id)
    }

    pub fn unload(&self, id: &str) -> Result<(), String> {
        Self::call(self.unload_fn, self.ctx, id)
    }

    pub fn show(&self, id: &str) -> Result<(), String> {
        Self::call(self.show_fn, self.ctx, id)
    }

    pub fn hide(&self, id: &str) -> Result<(), String> {
        Self::call(self.hide_fn, self.ctx, id)
    }
}
