//! Dedicated listen/dedicated server simulation thread (fixed tick rate).
//!
//! Owns the [`ServerThreadCtx`] facade — NOT a second Host: shared services
//! (loopback, cvar table, inspector share, enabled mirror) come from the
//! single main-thread Host as `Arc`s/handles, and console lines / command
//! registrations route back over [`ServerToMain`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use deerpunk_engine_common::cvar::CvarSystem;
use deerpunk_engine_common::network::NetLoopback;
use deerpunk_engine_common::resource::{MeshRegistry, ResourceManager};
use parking_lot::Mutex;

use crate::inspector::ServerInspectShare;
use crate::schedule::Schedule;
use crate::server_ctx::{ServerThreadCtx, ServerToMain};
use crate::ServerEngine;

pub struct ServerThread {
    running: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
    cmd_tx: crossbeam_channel::Sender<ServerThreadCmd>,
    server_to_host_rx: crossbeam_channel::Receiver<ServerToMain>,
}

pub enum ServerThreadCmd {
    LoadDll(String),
    SetEnabled(bool),
    /// `connect_local` + join init/catchup on the sim thread.
    ConnectLocal(String),
    /// Run a full console line — forwarded to the main-thread CmdSystem
    /// (single cmd system; server gamelib commands register there).
    ExecCmd(String),
    /// Listen host: main thread chose a world; gamelib loads it on this thread.
    StartMap {
        world: String,
        seed: i64,
    },
    Shutdown,
}

impl ServerThread {
    pub fn spawn(
        shared_loopback: Arc<NetLoopback>,
        gamedir: PathBuf,
        shared_cvar: Arc<CvarSystem>,
        enabled_flag: Arc<AtomicBool>,
        inspect_share: Arc<ServerInspectShare>,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (to_main_tx, to_main_rx) = crossbeam_channel::unbounded();

        let running_t = running.clone();
        let join = thread::Builder::new()
            .name("deerpunk-server".into())
            .spawn(move || {
                server_main(
                    shared_loopback,
                    gamedir,
                    shared_cvar,
                    enabled_flag,
                    inspect_share,
                    running_t,
                    cmd_rx,
                    to_main_tx,
                );
            })
            .expect("spawn server thread");

        Self {
            running,
            join: Mutex::new(Some(join)),
            cmd_tx,
            server_to_host_rx: to_main_rx,
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

    /// Main-thread receiver for [`ServerToMain`] messages (drained per frame).
    pub fn server_to_host_rx(&self) -> &crossbeam_channel::Receiver<ServerToMain> {
        &self.server_to_host_rx
    }

    pub fn load_dll(&self, path: String) {
        self.send(ServerThreadCmd::LoadDll(path));
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
    shared_cvar: Arc<CvarSystem>,
    enabled_flag: Arc<AtomicBool>,
    inspect_share: Arc<ServerInspectShare>,
    running: Arc<AtomicBool>,
    cmd_rx: crossbeam_channel::Receiver<ServerThreadCmd>,
    server_to_host: crossbeam_channel::Sender<ServerToMain>,
) {
    log::info!("[server-thread] simulation thread running");
    // Per-side registries (um ids match the client side by register order;
    // resources/mesh registry are RefCell-based, so the thread keeps its own).
    let mut resources = ResourceManager::new();
    let shader_root = std::env::var("SHADER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("shaders"));
    resources.register_loader(Box::new(deerpunk_engine_common::resource::FileLoader {
        root: shader_root.clone(),
    }));
    resources.register_loader(Box::new(deerpunk_engine_common::resource::TexgenLoader));
    resources.register_loader(Box::new(deerpunk_engine_common::resource::ShaderLoader {
        root: shader_root,
    }));

    let mut ctx = ServerThreadCtx {
        engine: ServerEngine::new(),
        loopback,
        cvar: shared_cvar,
        gamedir: gamedir.clone(),
        um: Default::default(),
        gamelib_schedule: Schedule::new(),
        resources,
        mesh_registry: MeshRegistry::new(),
        server_to_host,
        enabled_flag,
        inspect_share,
        console_commands: Vec::new(),
        inspect_world_ready: false,
        pending_start_map: None,
        last_overload_warn: None,
    };
    ctx.setup();

    let mut dll_loaded = false;
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
                    ctx.load_server_dll(&path);
                    dll_loaded = true;
                }
                ServerThreadCmd::StartMap { world, seed } => {
                    log::info!("[server-thread] StartMap '{world}' seed={seed}");
                    if dll_loaded {
                        ctx.apply_gamelib_start_map(&world, seed);
                    } else {
                        ctx.queue_start_map(world, seed);
                    }
                }
                ServerThreadCmd::SetEnabled(on) => {
                    ctx.apply_set_server_enabled(on);
                }
                ServerThreadCmd::ConnectLocal(name) => {
                    ctx.connect_local(name);
                }
                ServerThreadCmd::ExecCmd(line) => {
                    log::info!("[server-thread] exec (main → thread): {line}");
                    ctx.exec_console_line(&line);
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

        let tickrate = ctx.cvar.get_f32_or("sv_tickrate", 20.0).clamp(1.0, 240.0);
        let tick_dt = Duration::from_secs_f64(1.0 / tickrate as f64);

        let tick_budget_ms = tick_dt.as_secs_f64() * 1000.0;
        let profile = ctx.cvar.get_f32_or("sv_thread_profile", 0.0) != 0.0;
        let mut ticks_this_loop = 0u32;
        while tick_accum >= tick_dt {
            tick_accum -= tick_dt;
            let dt = tick_dt.as_secs_f32();
            let tick_t0 = Instant::now();
            ctx.tick(dt);
            ticks += 1;
            ticks_this_loop += 1;
            if profile {
                let wall_ms = tick_t0.elapsed().as_secs_f64() * 1000.0;
                if wall_ms > tick_budget_ms * 1.5 && ctx.warn_throttled() {
                    let accum_ms = tick_accum.as_secs_f64() * 1000.0;
                    eprintln!(
                        "[server-thread] Can't keep up! tick={ticks} wall={wall_ms:.1}ms budget={tick_budget_ms:.1}ms \
                         loop_ticks={ticks_this_loop} accum_ms={accum_ms:.1}"
                    );
                }
            }
            if ticks == 1 || ticks % 3600 == 0 {
                log::debug!(
                    "[server-thread] tick={ticks} sv_enabled={}",
                    ctx.enabled_flag.load(Ordering::Relaxed)
                );
            }
        }
        if profile && ticks_this_loop > 0 {
            let hz = ticks_this_loop as f64 / frame_dt.as_secs_f64().max(1e-6);
            if hz < tickrate as f64 * 0.75 && ctx.warn_throttled() {
                eprintln!(
                    "[server-thread] catch-up loop: frame_dt={:.1}ms ran {} tick(s) (~{:.1}hz, want {:.0}hz)",
                    frame_dt.as_secs_f64() * 1000.0,
                    ticks_this_loop,
                    hz,
                    tickrate
                );
            }
        }

        if !dll_loaded {
            thread::sleep(Duration::from_millis(1));
        } else {
            thread::sleep(Duration::from_micros(200));
        }
    }

    log::info!("[server-thread] shutting down");
    ctx.shutdown();
    log::info!("[server-thread] exit");
}
