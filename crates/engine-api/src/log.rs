use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type WriteFn = extern "C" fn(*mut u8, i32, *const u8, usize) -> i32;

pub struct LogApi {
    ctx: *mut u8,
    write_fn: WriteFn,
}

unsafe impl Send for LogApi {}
unsafe impl Sync for LogApi {}

impl LogApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            write_fn: unsafe { std::mem::transmute(resolve(r, "v1:log:write")) },
        }
    }

    /// `level`: 1=error, 2=warn, 3=info, 4=debug, 5=trace.
    pub fn write(&self, level: i32, msg: &str) -> bool {
        (self.write_fn)(self.ctx, level, msg.as_ptr(), msg.len()) == 0
    }
}
