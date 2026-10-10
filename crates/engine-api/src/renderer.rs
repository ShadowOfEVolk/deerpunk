use crate::resolve_fn;

pub struct RendererApiResolved {
    ctx: *mut u8,
    load_texture_fn: unsafe extern "C" fn(*mut u8, *const u8, usize) -> u32,
}

impl RendererApiResolved {
    pub fn new(ctx: *mut u8, resolver: crate::Resolver) -> Self {
        Self {
            ctx,
            load_texture_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:load_texture"))
            },
        }
    }

    pub fn load_texture(&self, path: &str) -> u32 {
        let bytes = path.as_bytes();
        unsafe { (self.load_texture_fn)(self.ctx, bytes.as_ptr(), bytes.len()) }
    }
}
