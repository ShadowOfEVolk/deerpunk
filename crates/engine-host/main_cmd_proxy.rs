//! Main-thread console proxies for commands registered by the server gamelib.
//!
//! Server `server_init` runs on the simulation thread and registers real handlers
//! on the server [`CmdSystem`]. It also calls [`enqueue_main_console_proxy`] so the
//! listen-client console / UI can forward lines to the server thread without the
//! engine knowing game command names.

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::cmd::{CommandRegistration, CmdSystem};
use crate::server_thread::ServerThreadCmd;

static SERVER_TX: Mutex<Option<Sender<ServerThreadCmd>>> = Mutex::new(None);
static PENDING_PROXY_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn attach(server_tx: Sender<ServerThreadCmd>) {
    *SERVER_TX.lock() = Some(server_tx);
}

/// Called from server gamelib via `v1:host:register_main_console` (server thread).
pub fn enqueue_main_console_proxy(name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    PENDING_PROXY_NAMES.lock().push(name.to_string());
    log::info!("[host] queued main-console proxy for '{name}'");
}

/// Register pending proxies on the main-thread [`CmdSystem`] (call each frame).
pub fn drain_pending_registrations(cmd: &mut CmdSystem) {
    let names: Vec<String> = PENDING_PROXY_NAMES.lock().drain(..).collect();
    for name in names {
        register_proxy(cmd, name);
    }
}

fn register_proxy(cmd: &mut CmdSystem, name: String) {
    let log_name = name.clone();
    let cmd_name = name;
    cmd.register(CommandRegistration::new(cmd_name.clone()).execute(move |ctx| {
        let line = if ctx.args.is_empty() {
            cmd_name.clone()
        } else {
            format!("{} {}", cmd_name, ctx.args.join(" "))
        };
        forward_to_server_thread(&line);
    }));
    log::info!("[host] main-console proxy registered: '{log_name}'");
}

pub fn forward_to_server_thread(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let tx = SERVER_TX.lock();
    match tx.as_ref() {
        Some(tx) => {
            log::info!("[host] main → server thread: {line}");
            let _ = tx.send(ServerThreadCmd::ExecCmd(line.to_string()));
        }
        None => {
            log::error!("[host] cannot forward '{line}': server thread not attached");
        }
    }
}
