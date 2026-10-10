use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type ClientCmdFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;
type SetEnabledFn = extern "C" fn(*mut u8, i32) -> i32;
type IsEnabledFn = extern "C" fn(*mut u8) -> i32;

pub struct NetApi {
    ctx: *mut u8,
    client_cmd_fn: ClientCmdFn,
    set_enabled_fn: SetEnabledFn,
    is_enabled_fn: IsEnabledFn,
}

unsafe impl Send for NetApi {}
unsafe impl Sync for NetApi {}

impl NetApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            client_cmd_fn: unsafe { std::mem::transmute(resolve(r, "v1:net:client_cmd")) },
            set_enabled_fn: unsafe { std::mem::transmute(resolve(r, "v1:server:set_enabled")) },
            is_enabled_fn: unsafe { std::mem::transmute(resolve(r, "v1:server:is_enabled")) },
        }
    }

    /// Queue client→server intent (`map <name>`, `disconnect`).
    pub fn client_cmd(&self, line: &str) -> Result<(), String> {
        let bytes = line.as_bytes();
        let rc = (self.client_cmd_fn)(self.ctx, bytes.as_ptr(), bytes.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("net.client_cmd failed: {line}"))
        }
    }

    pub fn set_server_enabled(&self, on: bool) {
        (self.set_enabled_fn)(self.ctx, if on { 1 } else { 0 });
    }

    pub fn server_enabled(&self) -> bool {
        (self.is_enabled_fn)(self.ctx) != 0
    }
}
