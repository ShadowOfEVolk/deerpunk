//! Console `listen` / `connect` / LAN — deferred to the net tick (no re-entrancy from cmd).

use std::sync::Mutex;

use deerpunk_engine_common::network::ClientSessionPhase;

use crate::cmd::CommandRegistration;
use crate::lan;
use crate::net;
use crate::Host;

pub(crate) enum PendingNetCmd {
    Listen(Option<u16>),
    Connect(String),
    NetDisconnect,
    LanRefresh,
    LanStop,
}

/// Drain the queued net console commands (main client host + sim thread both
/// call this; on listen clients the main host wins the race and forwards
/// `listen` / `net_disconnect` to the sim thread as commands).
pub(crate) fn take_pending() -> Vec<PendingNetCmd> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

static PENDING: Mutex<Vec<PendingNetCmd>> = Mutex::new(Vec::new());

pub fn register_commands(cmd: &mut crate::cmd::CmdSystem) {
    cmd.register(CommandRegistration::new("listen").execute(|ctx| {
        let port = ctx.args.first().and_then(|s| s.parse::<u16>().ok());
        PENDING.lock().unwrap().push(PendingNetCmd::Listen(port));
    }));
    cmd.register(CommandRegistration::new("connect").execute(|ctx| {
        let Some(addr) = ctx.args.first().copied() else {
            println!(
                "Usage: connect <ip[:port]>  (default port {})",
                net::DEFAULT_PORT
            );
            return;
        };
        PENDING
            .lock()
            .unwrap()
            .push(PendingNetCmd::Connect(addr.to_string()));
    }));
    cmd.register(CommandRegistration::new("net_disconnect").execute(|_| {
        PENDING.lock().unwrap().push(PendingNetCmd::NetDisconnect);
    }));
    cmd.register(CommandRegistration::new("lan_refresh").execute(|_| {
        PENDING.lock().unwrap().push(PendingNetCmd::LanRefresh);
    }));
    cmd.register(CommandRegistration::new("lan_stop").execute(|_| {
        PENDING.lock().unwrap().push(PendingNetCmd::LanStop);
    }));
}

impl Host {
    pub(crate) fn drain_pending_net_cmds(&mut self) {
        for cmd in take_pending() {
            match cmd {
                PendingNetCmd::Listen(port_opt) => {
                    let port = port_opt.unwrap_or_else(|| {
                        self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16
                    });
                    self.cvar.set_or_create("sv_port", &port.to_string());
                    // Enable + auto-listen + LAN advertise happen on the sim
                    // thread (main console never touches server net state).
                    if let Some(st) = &self.server_thread {
                        st.send(crate::server_thread::ServerThreadCmd::SetEnabled(true));
                        println!("[net] listen on server thread (port {port})");
                    }
                }
                PendingNetCmd::Connect(addr_s) => {
                    match net::parse_addr(&addr_s, net::DEFAULT_PORT) {
                        Ok(addr) => {
                            let port =
                                self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
                            #[cfg(any(feature = "client", feature = "full"))]
                            if self.client_engine.is_some()
                                && self.listen_server_enabled()
                                && net::is_connect_to_local_listen(addr, port)
                            {
                                println!("[net] connect {addr} → in-process (loopback)");
                                self.connect_to_server_inprocess();
                                continue;
                            }
                            if self.listen_server_enabled() {
                                self.set_server_enabled(false);
                            }
                            #[cfg(any(feature = "client", feature = "full"))]
                            {
                                self.flush_client_scene();
                                self.client_engine.as_mut().unwrap().join_phase =
                                    ClientSessionPhase::Idle;
                                match self.client_engine.as_mut().unwrap().network.connect(addr) {
                                    Ok(()) => {
                                        println!("[net] connecting to {addr}… (auth)");
                                    }
                                    Err(e) => println!("[net] connect failed: {e}"),
                                }
                            }
                        }
                        Err(e) => println!("[net] connect: {e}"),
                    }
                }
                PendingNetCmd::NetDisconnect => {
                    self.disconnect_all_net();
                    println!("[net] net_disconnect");
                }
                PendingNetCmd::LanRefresh =>
                {
                    #[cfg(any(feature = "client", feature = "full"))]
                    if let Some(client) = self.client_engine.as_mut() {
                        let port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
                        match client.lan_browser.ensure_listening(port) {
                            Ok(()) => {
                                client.lan_browser.send_query();
                                println!("[lan] refresh port={port}");
                            }
                            Err(e) => println!("[lan] refresh failed: {e}"),
                        }
                    }
                }
                PendingNetCmd::LanStop =>
                {
                    #[cfg(any(feature = "client", feature = "full"))]
                    if let Some(client) = self.client_engine.as_mut() {
                        client.lan_browser.stop();
                        lan::set_servers_json(&[]);
                        deerpunk_ui::set_lan_servers_json("[]");
                    }
                }
            }
        }
    }
}
