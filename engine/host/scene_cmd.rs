//! `scene <name>` console command → deferred load on the host/main thread.

use std::sync::Mutex;

use crate::cmd::{CommandExecutionContext, CmdSystem};

static PENDING: Mutex<Option<String>> = Mutex::new(None);

pub fn register(cmd: &mut CmdSystem) {
    cmd.register_command("scene", |ctx: CommandExecutionContext<'_>| {
        if ctx.args.is_empty() {
            println!("Usage: scene <name>   (e.g. scene game)");
            return;
        }
        let name = ctx.args[0].trim().to_string();
        if name.is_empty() {
            println!("Usage: scene <name>");
            return;
        }
        match PENDING.lock() {
            Ok(mut slot) => {
                println!("[cmd] scene {name} (queued)");
                *slot = Some(name);
            }
            Err(_) => println!("[cmd] scene: lock poisoned"),
        }
    });
}

pub fn take_pending() -> Option<String> {
    PENDING.lock().ok().and_then(|mut g| g.take())
}
