use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct SceneApi {
    ctx: *mut u8,
    load_fn: extern "C" fn(*mut u8, *const u8, usize) -> i32,
}

unsafe impl Send for SceneApi {}
unsafe impl Sync for SceneApi {}

impl SceneApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            load_fn: unsafe { std::mem::transmute(resolve(r, "v1:scene:load")) },
        }
    }

    /// Load `{gamedir}/scenes/{name}.scene` on the server.
    pub fn load(&self, name: &str) -> Result<(), String> {
        let bytes = name.as_bytes();
        let rc = (self.load_fn)(self.ctx, bytes.as_ptr(), bytes.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("scene load failed: {name}"))
        }
    }
}
