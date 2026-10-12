//! Console command registration from gamelib (`v1:cmd:*`).

use std::ffi::c_void;

use crate::cmd::CommandExecutionContext;
use crate::engine_api::engine_api::EngineApiMethod;

/// Gamelib console callback: `argc`, argv ptrs/lens (UTF-8), user data.
pub type CmdCallback =
    extern "C" fn(i32, *const *const u8, *const usize, *mut c_void);

struct DllCmd {
    callback: CmdCallback,
    user: *mut c_void,
}

unsafe impl Send for DllCmd {}
unsafe impl Sync for DllCmd {}

fn read_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn invoke_dll_cmd(dll: &DllCmd, args: &[&str]) {
    let argc = args.len() as i32;
    let ptrs: Vec<*const u8> = args.iter().map(|s| s.as_ptr()).collect();
    let lens: Vec<usize> = args.iter().map(|s| s.len()).collect();
    (dll.callback)(argc, ptrs.as_ptr(), lens.as_ptr(), dll.user);
}

/// `v1:cmd:register` — register a console command handled by gamelib.
pub extern "C" fn tramp_cmd_register(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    callback: CmdCallback,
    user: *mut c_void,
) -> i32 {
    let name = read_str(name_ptr, name_len);
    let name = name.trim();
    if name.is_empty() {
        log::error!("[cmd] register: empty name");
        return -1;
    }
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let dll = DllCmd { callback, user };
    let name_owned = name.to_string();
    host.cmd.register_command(&name_owned, move |ctx: CommandExecutionContext<'_>| {
        invoke_dll_cmd(&dll, &ctx.args);
    });
    log::info!("[cmd] registered gamelib command '{name_owned}'");
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:cmd:register",
        callback: tramp_cmd_register as *const (),
        flags: 0,
    }
}
