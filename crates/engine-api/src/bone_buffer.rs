use std::ffi::c_void;

use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> Option<*const ()> {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

pub struct BoneBufferApi {
    ctx: *mut u8,
    alloc_fn: Option<extern "C" fn(*mut u8, u32) -> u32>,
    free_fn: Option<extern "C" fn(*mut u8, u32)>,
    write_fn: Option<extern "C" fn(*mut u8, u32, *const f32, u32) -> i32>,
}

unsafe impl Send for BoneBufferApi {}
unsafe impl Sync for BoneBufferApi {}

impl BoneBufferApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            alloc_fn: resolve(r, "v1:bone_buffer:alloc").map(|p| unsafe { std::mem::transmute(p) }),
            free_fn: resolve(r, "v1:bone_buffer:free").map(|p| unsafe { std::mem::transmute(p) }),
            write_fn: resolve(r, "v1:bone_buffer:write")
                .map(|p| unsafe { std::mem::transmute(p) }),
        }
    }

    pub fn alloc(&self, bone_count: u32) -> u32 {
        let Some(f) = self.alloc_fn else {
            return 0;
        };
        f(self.ctx, bone_count)
    }

    pub fn free(&self, handle: u32) {
        if let Some(f) = self.free_fn {
            f(self.ctx, handle);
        }
    }

    /// `matrices` — column-major `16 * bone_count` floats.
    pub fn write(&self, handle: u32, matrices: &[f32], bone_count: u32) -> bool {
        let Some(f) = self.write_fn else {
            return false;
        };
        f(self.ctx, handle, matrices.as_ptr(), bone_count) != 0
    }
}
