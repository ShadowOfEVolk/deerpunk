use std::ffi::c_void;

use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

/// Gamelib console callback: argc + argv (UTF-8 ptr/len pairs) + user data.
pub type CmdCallback = extern "C" fn(i32, *const *const u8, *const usize, *mut c_void);

type RegisterFn = extern "C" fn(*mut u8, *const u8, usize, CmdCallback, *mut c_void) -> i32;
type ExecuteFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;

pub struct CmdApi {
    ctx: *mut u8,
    register_fn: RegisterFn,
    execute_fn: ExecuteFn,
}

unsafe impl Send for CmdApi {}
unsafe impl Sync for CmdApi {}

impl CmdApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            register_fn: unsafe { std::mem::transmute(resolve(r, "v1:cmd:register")) },
            execute_fn: unsafe { std::mem::transmute(resolve(r, "v1:cmd:execute")) },
        }
    }

    /// Register a console command handled by `callback`.
    pub fn register(&self, name: &str, callback: CmdCallback, user: *mut c_void) -> Result<(), String> {
        let bytes = name.as_bytes();
        let rc = (self.register_fn)(self.ctx, bytes.as_ptr(), bytes.len(), callback, user);
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("cmd.register failed: {name}"))
        }
    }

    pub fn execute(&self, command: &str) -> Result<(), String> {
        let bytes = command.as_bytes();
        let rc = (self.execute_fn)(self.ctx, bytes.as_ptr(), bytes.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("cmd.execute failed: {command}"))
        }
    }
}
