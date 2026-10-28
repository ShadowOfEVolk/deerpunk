//! Dedicated server simulation thread (fixed tick rate). Owns a server-side [`Host`] instance.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use deerpunk_engine_common::network::NetLoopback;
use parking_lot::Mutex;

use crate::Host;

deerpunk_engine_common::cvar_def!(
    "sv_tickrate",
    "60",
    "Server simulation ticks per second (fixed timestep).",
    deerpunk_engine_common::cvar_def::ARCHIVE
);

deerpunk_engine_common::cvar_def!(
    "sv_thread_profile",
    "1",
    "Log server-thread ticks slower than 1.5x sv_tickrate budget (default on, throttled to ~5s).",
    deerpunk_engine_common::cvar_def::ARCHIVE
);

pub struct ServerThread {
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
    cmd_tx: crossbeam_channel::Sender<ServerThreadCmd>,
}

pub enum ServerThreadCmd {
    LoadDll(String),
    SetEnabled(bool),
    /// `connect_local` + join init/catchup on the server thread.
    ConnectLocal(String),
    /// Run a full console line on the server-thread [`CmdSystem`].
    ExecCmd(String),
    /// Listen host: main thread chose a world; gamelib loads it on this thread.
    StartMap {
        world: String,
        seed: i64,
    },
    FinishJoin(Vec<u32>),
    Shutdown,
}

impl ServerThread {
    pub fn spawn(
        shared_loopback: Arc<NetLoopback>,
        gamedir: PathBuf,
        shared_cvar: Arc<crate::cvar::CvarSystem>,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let enabled = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();

        let running_t = running.clone();
        let enabled_t = enabled.clone();
        let join = thread::Builder::new()
            .name("deerpunk-server".into())
            .spawn(move || {
                server_main(
                    shared_loopback,
                    gamedir,
                    shared_cvar,
                    running_t,
                    enabled_t,
                    cmd_rx,
                );
            })
            .expect("spawn server thread");

        Self {
            running,
            enabled,
            join: Mutex::new(Some(join)),
            cmd_tx,
        }
    }

    pub fn send(&self, cmd: ServerThreadCmd) {
        if let Err(e) = self.cmd_tx.send(cmd) {
            log::error!("[server-thread] cmd send failed: {e}");
        }
    }

    pub fn cmd_sender(&self) -> &crossbeam_channel::Sender<ServerThreadCmd> {
        &self.cmd_tx
    }

    pub fn load_dll(&self, path: String) {
        self.send(ServerThreadCmd::LoadDll(path));
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled_flag(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.cmd_tx.send(ServerThreadCmd::Shutdown);
        if let Some(j) = self.join.lock().take() {
            let _ = j.join();
        }
    }
}

fn server_main(
    loopback: Arc<NetLoopback>,
    gamedir: PathBuf,
    shared_cvar: Arc<crate::cvar::CvarSystem>,
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    cmd_rx: crossbeam_channel::Receiver<ServerThreadCmd>,
) {
    log::info!("[server-thread] simulation thread running");
    let mut host = Host::new_server_instance(loopback, gamedir.clone(), shared_cvar);
    host.set_listen_enabled_sink(enabled.clone());
    host.server_initialize(&gamedir);

    let mut dll_loaded = false;
    let mut pending_start_map: Option<(String, i64)> = None;
    let mut ticks: u64 = 0;
    let mut tick_accum = Duration::ZERO;
    let mut last = Instant::now();
    // Throttle for the "Can't keep up!" overload warnings so a sustained
    // slowdown doesn't flood the log every tick.
    let mut last_overload_warn: Option<Instant> = None;

    'server_loop: while running.load(Ordering::Relaxed) {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ServerThreadCmd::LoadDll(path) => {
                    log::info!("[server-thread] loading gamelib: {path}");
                    host.load_server_dll_on_thread(&path);
                    host.finish_server_dll_startup();
                    dll_loaded = true;
                    log::info!("[server-thread] gamelib loaded: {path}");
                    if let Some((world, seed)) = pending_start_map.take() {
                        host.apply_gamelib_start_map(&world, seed);
                    }
                }
                ServerThreadCmd::StartMap { world, seed } => {
                    log::info!("[server-thread] StartMap '{world}' seed={seed}");
                    if dll_loaded {
                        host.apply_gamelib_start_map(&world, seed);
                    } else {
                        pending_start_map = Some((world, seed));
                    }
                }
                ServerThreadCmd::SetEnabled(on) => {
                    host.apply_set_server_enabled(on);
                }
                ServerThreadCmd::ConnectLocal(name) => {
                    if host.server_mut().players.connect_local(name).is_some() {
                        let slot = host.server_mut().players.local_slot().unwrap_or(0);
                        host.finish_client_join(&[slot]);
                    }
                }
                ServerThreadCmd::ExecCmd(line) => {
                    log::info!("[server-thread] exec: {line}");
                    host.cmd.add_command(line);
                    host.rebind_cmd_refs();
                    host.cmd.frame();
                }
                ServerThreadCmd::FinishJoin(slots) => {
                    host.finish_client_join(&slots);
                }
                ServerThreadCmd::Shutdown => {
                    running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }

        if !running.load(Ordering::Relaxed) {
            break 'server_loop;
        }

        let now = Instant::now();
        let frame_dt = now - last;
        last = now;
        tick_accum += frame_dt;

        let tickrate = host.cvar.get_f32_or("sv_tickrate", 20.0).clamp(1.0, 240.0);
        let tick_dt = Duration::from_secs_f64(1.0 / tickrate as f64);

        let tick_budget_ms = tick_dt.as_secs_f64() * 1000.0;
        let profile = host.cvar.get_f32_or("sv_thread_profile", 0.0) != 0.0;
        let mut ticks_this_loop = 0u32;
        while tick_accum >= tick_dt {
            tick_accum -= tick_dt;
            let dt = tick_dt.as_secs_f32();
            let tick_t0 = Instant::now();
            host.server_thread_tick(dt, &enabled);
            ticks += 1;
            ticks_this_loop += 1;
            if profile {
                let wall_ms = tick_t0.elapsed().as_secs_f64() * 1000.0;
                if wall_ms > tick_budget_ms * 1.5 {
                    let now_w = Instant::now();
                    let should_warn = match last_overload_warn {
                        None => true,
                        Some(t) => now_w.duration_since(t).as_secs_f64() >= 5.0,
                    };
                    if should_warn {
                        last_overload_warn = Some(now_w);
                        let accum_ms = tick_accum.as_secs_f64() * 1000.0;
                        eprintln!(
                            "[server-thread] Can't keep up! tick={ticks} wall={wall_ms:.1}ms budget={tick_budget_ms:.1}ms \
                             loop_ticks={ticks_this_loop} accum_ms={accum_ms:.1}"
                        );
                    }
                }
            }
            if ticks == 1 || ticks % 3600 == 0 {
                log::debug!(
                    "[server-thread] tick={ticks} sv_enabled={}",
                    enabled.load(Ordering::Relaxed)
                );
            }
        }
        if profile && ticks_this_loop > 0 {
            let hz = ticks_this_loop as f64 / frame_dt.as_secs_f64().max(1e-6);
            if hz < tickrate as f64 * 0.75 {
                let now_w = Instant::now();
                let should_warn = match last_overload_warn {
                    None => true,
                    Some(t) => now_w.duration_since(t).as_secs_f64() >= 5.0,
                };
                if should_warn {
                    last_overload_warn = Some(now_w);
                    eprintln!(
                        "[server-thread] catch-up loop: frame_dt={:.1}ms ran {} tick(s) (~{:.1}hz, want {:.0}hz)",
                        frame_dt.as_secs_f64() * 1000.0,
                        ticks_this_loop,
                        hz,
                        tickrate
                    );
                }
            }
        }

        if !dll_loaded {
            thread::sleep(Duration::from_millis(1));
        } else {
            thread::sleep(Duration::from_micros(200));
        }
    }

    log::info!("[server-thread] shutting down");
    host.server_shutdown();
    log::info!("[server-thread] exit");
}
