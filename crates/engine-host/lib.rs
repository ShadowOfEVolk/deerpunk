#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
pub type Entity = u64;

pub mod gamelib_abi;
pub mod gamelib_system;
pub use gamelib_abi::{ScheduleSystemFn, WorldQueryCallback};

pub mod binds;
mod client_sync;
pub mod cmd;
pub mod cmd_completion;
pub mod config_io;
pub mod console;
pub mod cvar;
#[cfg(any(feature = "client", feature = "full"))]
mod demo_console;
mod engine_methoddb;
pub mod fs;
pub mod gameinfo;
pub mod inspector;
mod mesh_components;
mod mesh_upload;
pub mod move_state;
mod net_console;
mod net_tick;
mod particles_net;
mod particles_spawn;
mod physics_trace;
pub mod schedule;
mod server_ctx;
mod server_thread;
mod spawn_prefab;
pub mod stdin_cmd;
pub mod system_profiler;
mod world_host_api;
pub use schedule::Schedule;

#[cfg(any(feature = "client", feature = "full"))]
pub use deerpunk_engine_client::{ClientDll, ClientInitData};
pub use deerpunk_engine_common::network::{ClientSessionPhase, SlotPlayGate};
pub use deerpunk_engine_server::lan_advertiser::LanAdvertiser;
pub use deerpunk_engine_server::{
    PlayerConnection, PlayerConnections, ServerDll, ServerEngine, ServerInitData,
};

pub mod api;
pub use api::{ComponentId, EntityId};

pub mod engine_api;
pub use deerpunk_engine_common::world;

pub use crate::cmd::CmdCallback;
use deerpunk_engine_common::network::decode_client_command_payload;
use deerpunk_engine_common::network::protocol::{CLC, SVC};
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};
use deerpunk_engine_common::network::user_message::UserMessageRpc;
use deerpunk_engine_common::network::{UserCmd, UserCmdAckRpc};
pub use engine_api::ui::*;

mod net_packet_log;

pub mod net {
    pub use deerpunk_engine_server::net::*;
}
pub mod connect_auth {
    pub use deerpunk_engine_server::connect_auth::*;
}
pub mod lan {
    #[cfg(any(feature = "client", feature = "full"))]
    pub use deerpunk_engine_client::lan_browser::*;
    pub use deerpunk_engine_common::network::lan::*;
    pub use deerpunk_engine_server::lan_advertiser::LanAdvertiser;
}

pub mod user_messages;
pub use deerpunk_engine_common::network::{HostCommand, NetLoopback};
pub use deerpunk_engine_ffi::ResolverFn;
use std::sync::Arc;
use std::time::Duration;

#[cfg(any(feature = "client", feature = "full"))]
pub mod entity_debug {
    pub use deerpunk_engine_client::entity_debug::*;
}
#[cfg(any(feature = "client", feature = "full"))]
pub mod phys_debug {
    pub use deerpunk_engine_client::phys_debug::*;
}
#[cfg(any(feature = "client", feature = "full"))]
pub mod demo {
    pub use deerpunk_engine_client::demo::*;
}
pub mod players {
    pub use deerpunk_engine_server::players::*;
}
pub mod server_engine {
    pub use deerpunk_engine_server::server_engine::*;
}
pub mod server_dll {
    pub use deerpunk_engine_server::server_dll::*;
}
#[cfg(any(feature = "client", feature = "full"))]
pub mod client_dll {
    pub use deerpunk_engine_client::client_dll::*;
}
pub mod connection {
    pub use deerpunk_engine_common::network::{ClientSessionPhase, SlotPlayGate};
}

pub mod common {
    pub use deerpunk_engine_common::*;
}

#[cfg(feature = "dedicated-server")]
pub mod server {
    pub use deerpunk_engine_server::*;
}

#[cfg(feature = "client")]
pub mod client {
    pub use deerpunk_engine_client::*;
}

#[cfg(any(feature = "client", feature = "full"))]
extern "C" fn host_frame_ui_trampoline(ctx: *mut std::ffi::c_void) {
    let host = unsafe { &mut *(ctx as *mut Host) };
    host.frame_ui();
}

// ── Host ─────────────────────────────────────────────────────
//
// Foundation: cvar/cmd/fs/resources/mesh_registry/inspector/time — all on
//             this main-thread instance (there is NO second Host).
// Server:     [`ServerEngine`] lives on the sim thread behind the
//             [`server_ctx::ServerThreadCtx`] facade (Arcs + channels only);
//             reached from here via [`Self::server_thread`] commands and the
//             enabled mirror.
// Client:     [`client_engine`] — render/input + session (DLL, console, rep, demo).
// Glue:       `loopback` (in-process CLC/SVC queues), `net` (UDP net socket + renet), `um`.

pub struct Host {
    /// The ONE Host lives on the main thread; the server simulation
    /// ([`ServerEngine`]) runs on the sim thread through the
    /// [`server_ctx::ServerThreadCtx`] facade. No second Host exists.
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<ClientEngine>,
    /// Shared listen-server wire queues (in-memory transport).
    pub loopback: Arc<NetLoopback>,
    /// Simulation thread handle (owns the server engine via the facade).
    pub(crate) server_thread: Option<server_thread::ServerThread>,
    /// Internally locked table, shared by value clones (main + sim thread).
    pub cvar: Arc<cvar::CvarSystem>,
    pub cmd: cmd::CmdSystem,
    pub fs: fs::FileSystem,
    pub gameinfo: gameinfo::GameInfo,
    pub resources: deerpunk_engine_common::resource::ResourceManager,
    /// Shared mesh handles for client draw (server spawn uses the facade's).
    pub mesh_registry: deerpunk_engine_common::resource::MeshRegistry,
    /// Name → id map for custom mesh allocators registered via
    /// `v1:mesh:register_allocator`. Id `0` is reserved for the built-in
    /// slab allocator and is never stored here.
    pub mesh_allocator_names: std::collections::HashMap<String, u32>,
    /// Main-side user-message registry (client hooks; ids match the sim
    /// thread's registry by registration order).
    pub um: user_messages::UserMessageRegistry,
    pub inspector: inspector::Inspector,
    /// Gamelib / host load UI (`v1:host:loader_queue_set`).
    pub loader_queue: deerpunk_engine_common::loader_queue::LoaderQueue,
    pub(crate) time_secs: f32,
    /// Sound ids when no server engine is reachable (client-only host).
    next_client_sound_id: u32,
    /// Mirrored `enabled` state of the sim-thread engine (written by the
    /// facade each tick; read from the main thread without crossing over).
    server_enabled_flag: Arc<std::sync::atomic::AtomicBool>,
    /// Server-world inspector snapshots (published by the sim thread).
    server_inspector_share: Arc<inspector::ServerInspectShare>,
}

#[cfg(any(feature = "client", feature = "full"))]
impl Host {
    pub fn client_engine(&mut self) -> Option<&mut ClientEngine> {
        self.client_engine.as_mut()
    }

    pub fn client_engine_ref(&self) -> Option<&ClientEngine> {
        self.client_engine.as_ref()
    }

    /// Borrow the client engine and the mesh registry as disjoint mutable
    /// references (they are separate fields), so the engine can `sweep` the
    /// registry during `extract_scene_view`.
    fn split_client_registry(
        &mut self,
    ) -> (
        &mut ClientEngine,
        &mut deerpunk_engine_common::resource::MeshRegistry,
    ) {
        let ce = self.client_engine.as_mut().expect("client engine");
        (ce, &mut self.mesh_registry)
    }
}

impl Host {
    fn client_net_active(&self) -> bool {
        #[cfg(any(feature = "client", feature = "full"))]
        {
            return self
                .client_engine
                .as_ref()
                .is_some_and(|c| c.network.is_active());
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            false
        }
    }

    fn disconnect_all_net(&mut self) {
        if let Some(st) = &self.server_thread {
            st.send(server_thread::ServerThreadCmd::SetEnabled(false));
        }
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            client.network.disconnect();
        }
    }

    pub fn new() -> Self {
        // Do not set_refs here: returning Self moves cvar/fs and would leave
        // dangling pointers inside cmd. Rebind in `rebind_cmd_refs` after place is stable.
        let mut resources = deerpunk_engine_common::resource::ResourceManager::new();
        // FileLoader must exist before client loads WGSL through the asset manager.
        let shader_root = std::env::var("SHADER_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("shaders"));
        resources.register_loader(Box::new(deerpunk_engine_common::resource::FileLoader {
            root: shader_root.clone(),
        }));
        resources.register_loader(Box::new(deerpunk_engine_common::resource::TexgenLoader));
        resources.register_loader(Box::new(deerpunk_engine_common::resource::ShaderLoader {
            root: shader_root,
        }));
        #[cfg(any(feature = "tracy", feature = "puffin"))]
        deerpunk_engine_renderer::profile_ensure_started();
        #[cfg(any(feature = "client", feature = "full"))]
        let client_engine = Some(ClientEngine::new(&resources));
        let loopback = Arc::new(NetLoopback::new());
        Self {
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine,
            loopback,
            server_thread: None,
            cvar: Arc::new(cvar::CvarSystem::new()),
            cmd: cmd::CmdSystem::new(),
            fs: fs::FileSystem::new(),
            gameinfo: gameinfo::GameInfo::from_gamedir(std::path::Path::new(".")),
            resources,
            mesh_registry: deerpunk_engine_common::resource::MeshRegistry::new(),
            mesh_allocator_names: std::collections::HashMap::new(),
            um: user_messages::UserMessageRegistry::default(),
            inspector: inspector::Inspector::new(),
            loader_queue: deerpunk_engine_common::loader_queue::LoaderQueue::default(),
            time_secs: 0.0,
            next_client_sound_id: 1,
            server_enabled_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            server_inspector_share: Arc::new(inspector::ServerInspectShare::new()),
        }
    }

    /// Mirrored sim-thread `engine.enabled` (written by the facade each tick;
    /// the main thread reads it without touching the engine).
    fn listen_server_enabled(&self) -> bool {
        self.server_enabled_flag
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Forward the gamelib path to the sim thread (load + `init` happen there,
    /// against the facade ctx). Configs + inspector start on this thread once
    /// [`ServerToMain::ServerDllStarted`] arrives.
    pub fn load_server_dll(&mut self, path: &str) {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(st) = &self.server_thread {
            st.load_dll(path.to_string());
            return;
        }
        let _ = path;
        log::error!("[host] load_server_dll: no server thread");
    }

    /// Main-thread half of the server gamelib startup: exec `{gamename}.rc`
    /// and start the inspector WS (after `server_init` registered cvars/ums).
    pub(crate) fn finish_server_dll_startup(&mut self) {
        self.exec_game_rc();
        self.start_inspector();
    }

    /// Listen host: queue a world load on the sim thread (thread defers it
    /// until the gamelib is loaded).
    pub(crate) fn request_listen_map(&self, world: &str, seed: i64) -> i32 {
        let world = world.trim();
        if world.is_empty() {
            log::warn!("[host] start_listen_map: empty world name");
            return -1;
        }
        let Some(st) = &self.server_thread else {
            log::error!("[host] start_listen_map: no server thread");
            return -1;
        };
        log::info!("[host] start_listen_map '{world}' (main → server thread)");
        if let Err(e) = st
            .cmd_sender()
            .send(server_thread::ServerThreadCmd::StartMap {
                world: world.to_string(),
                seed,
            })
        {
            log::error!("[host] start_listen_map send failed: {e}");
            return -1;
        }
        0
    }

    /// Main-thread inspector poll: one poll serves the client world
    /// synchronously and the server world via the sim-thread share.
    fn poll_server_inspector(&mut self) {
        if !self.inspector.is_running() {
            return;
        }
        let time_secs = self.time_secs;
        let resources = &self.resources;
        let ctx = inspector::InspectorRpcCtx {
            resources,
            time_secs,
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine: self.client_engine.as_ref(),
            server: &self.server_inspector_share,
        };
        self.inspector.poll(Some(ctx));
    }

    /// CmdSystem holds raw pointers into Host fields — refresh after any move.
    fn rebind_cmd_refs(&mut self) {
        let cmd = &mut self.cmd;
        let cvar = self.cvar.as_ref();
        let fs = &self.fs;
        cmd.set_refs(cvar, fs);
        #[cfg(any(feature = "client", feature = "full"))]
        cmd.set_client_ref(self.client_engine.as_mut());
        #[cfg(any(feature = "client", feature = "full"))]
        cmd.set_resources_ref(Some(&self.resources));
    }

    pub fn initialize(&mut self, gamedir: impl AsRef<std::path::Path>) {
        self.rebind_cmd_refs();
        self.fs.initialize();
        self.fs.set_gamedir(&gamedir);
        #[cfg(any(feature = "client", feature = "full"))]
        deerpunk_ui::set_content_root(gamedir.as_ref());
        deerpunk_engine_common::resource::register_default_loaders(
            &self.resources,
            gamedir.as_ref(),
        );
        self.cvar.ensure_initialized();
        self.cmd.initialize();
        binds::Binds::register_commands(&mut self.cmd);
        move_state::register_commands(&mut self.cmd);
        net_console::register_commands(&mut self.cmd);
        crate::particles_net::register_commands(&mut self.cmd);
        #[cfg(any(feature = "client", feature = "full"))]
        demo_console::register_commands(&mut self.cmd);
        self.cmd.set_cmdline(std::env::args().collect());
        self.gameinfo = gameinfo::GameInfo::from_gamedir(gamedir.as_ref());
        self.cvar
            .set_or_create("gamename", &self.gameinfo.gamefolder);
        self.cvar
            .set_or_create("gamedir", &gamedir.as_ref().to_string_lossy());
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            let title = self.gameinfo.window_title().to_string();
            let icon = self.gameinfo.icon_path(gamedir.as_ref());
            client.apply_window_identity(&title, icon.as_deref());
        }
        // Configs + stuffcmds live in `{gamename}.rc`, exec'd after gamelibs.
        self.register_internal_components();

        // UI runtime before gamelib so `v1:ui:*` works from server_init.
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let gamedir_ui = self.fs.resolve_ui_dir();
            if let Some(client) = self.client_engine.as_mut() {
                client.init_ui(gamedir_ui);
            }
            let gamedir = gamedir.as_ref().to_path_buf();
            if let Some(client) = self.client_engine.as_mut() {
                #[cfg(feature = "audio-kira")]
                let backend: Box<dyn deerpunk_engine_client::AudioBackend> =
                    Box::new(deerpunk_engine_client_kira::KiraBackend::default());
                #[cfg(all(feature = "audio-fmod", not(feature = "audio-kira")))]
                let backend: Box<dyn deerpunk_engine_client::AudioBackend> =
                    Box::new(deerpunk_engine_client_fmod::FmodBackend::default());
                #[cfg(not(any(feature = "audio-fmod", feature = "audio-kira")))]
                let backend: Box<dyn deerpunk_engine_client::AudioBackend> =
                    Box::new(deerpunk_engine_client::NullAudio);
                client.set_audio_backend(backend, &gamedir, &self.resources);
                // Engine particle effects: `<gamedir>/particles/*.json`.
                client.init_particles_catalog(&gamedir);
            }
            self.rebind_cmd_refs();
        }

        // Sim thread owns the server engine behind the facade; this Host
        // shares loopback/cvar and mirrors enabled state + inspector data.
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let server_thread = server_thread::ServerThread::spawn(
                self.loopback.clone(),
                gamedir.as_ref().to_path_buf(),
                self.cvar.clone(),
                self.server_enabled_flag.clone(),
                self.server_inspector_share.clone(),
            );
            self.server_thread = Some(server_thread);
            log::info!("[host] server simulation thread spawned");
        }

        stdin_cmd::start();

        log::info!("Engine initialized");
    }

    /// Start inspector WS using current cvars (after configs / stuffcmds).
    pub fn start_inspector(&mut self) {
        // Runtime/definition descriptors must be registered on the Host that
        // owns the listening inspector — this single Host — right before the
        // bind (one registration pass; collection ids replace by id).
        self.inspector
            .register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
                id: "assets".into(),
                label: "Assets".into(),
                group: deerpunk_inspector_protocol::CollectionGroup::Runtime,
                modes: vec![
                    deerpunk_inspector_protocol::CollectionMode::List,
                    deerpunk_inspector_protocol::CollectionMode::Get,
                ],
                id_field: "index".into(),
                nested_in: None,
            });
        self.inspector
            .register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
                id: "systems".into(),
                label: "Frame profile".into(),
                group: deerpunk_inspector_protocol::CollectionGroup::Runtime,
                modes: vec![deerpunk_inspector_protocol::CollectionMode::List],
                id_field: "name".into(),
                nested_in: None,
            });
        #[cfg(any(feature = "client", feature = "full"))]
        self.inspector
            .register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
                id: "fmod_events".into(),
                label: "FMOD Events".into(),
                group: deerpunk_inspector_protocol::CollectionGroup::Runtime,
                modes: vec![
                    deerpunk_inspector_protocol::CollectionMode::List,
                    deerpunk_inspector_protocol::CollectionMode::Get,
                ],
                id_field: "index".into(),
                nested_in: None,
            });
        self.inspector
            .register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
                id: "players".into(),
                label: "Players".into(),
                group: deerpunk_inspector_protocol::CollectionGroup::Runtime,
                modes: vec![
                    deerpunk_inspector_protocol::CollectionMode::List,
                    deerpunk_inspector_protocol::CollectionMode::Get,
                ],
                id_field: "index".into(),
                nested_in: None,
            });
        self.inspector
            .register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
                id: "methoddb".into(),
                label: "MethodDB".into(),
                group: deerpunk_inspector_protocol::CollectionGroup::Definitions,
                modes: vec![
                    deerpunk_inspector_protocol::CollectionMode::List,
                    deerpunk_inspector_protocol::CollectionMode::Get,
                ],
                id_field: "index".into(),
                nested_in: None,
            });
        // Force queued CLI overrides (`--inspector-*`) over config.cfg values.
        self.cvar.apply_overrides();
        let port = self
            .cvar
            .get_string("sv_inspector_port")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(9000);
        let host = self
            .cvar
            .get_string("sv_inspector_host")
            .unwrap_or_else(|| "127.0.0.1".into());
        log::info!("[inspector] start host={host} port={port}");
        self.inspector.start(&host, port);
    }

    fn register_internal_components(&mut self) {
        use deerpunk_engine_common::components::*;
        use deerpunk_engine_common::physics::Collider;
        let register = |w: &deerpunk_engine_common::world::World| {
            deerpunk_engine_common::scene::register_engine_prefab_appliers(w);
            deerpunk_engine_common::scene::register_engine_component_inspect(w);
            w.component_init::<Terrain>();
            w.component_init::<TerrainSplatMaterial>();
            w.component_init::<Worldspawn>();
            w.component_init::<MapKv>();
            w.component_init::<Collider>();
            w.component_init::<ParticleEmitter>();
        };
        // Server-world registration happens on the sim thread
        // (`ServerThreadCtx::setup`); this registers the client world.
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            register(&client.world);
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            let _ = &register;
        }
    }

    pub fn method_db(&self) -> engine_api::engine_api::MethodDB {
        engine_api::engine_api::MethodDB::new()
    }

    /// Exec `{gamedir}/{gamename}.rc`. Fallback: configs + stuffcmds if missing.
    pub fn exec_game_rc(&self) {
        let name = self.gameinfo.gamefolder.as_str();
        if name.is_empty() {
            log::warn!("[host] gamename empty — fallback configs + stuffcmds");
            config_io::exec_startup(&self.cmd, &self.fs);
            self.cmd.execute_command("stuffcmds");
            return;
        }
        let rel = format!("{name}.rc");
        let path = self.fs.resolve_path(&rel);
        if path.is_file() {
            log::info!("[host] exec {}", path.display());
            self.cmd.exec_script_file(&rel);
        } else {
            log::warn!(
                "[host] {} missing — fallback configs + stuffcmds",
                path.display()
            );
            config_io::exec_startup(&self.cmd, &self.fs);
            self.cmd.execute_command("stuffcmds");
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn load_client_dll(&mut self, path: &str) {
        let ctx_ptr = self as *mut Host as *mut u8;
        let db = self.method_db();
        let resolver = db.into_resolver();

        match ClientDll::load(path) {
            Ok(client) => {
                unsafe {
                    client.init(ctx_ptr, resolver);
                }
                if let Some(ce) = self.client_engine.as_mut() {
                    ce.set_audio_world_scene_hook(client.audio_world_scene_fn());
                }
                self.client_engine.as_mut().unwrap().dll = Some(client);
                log::info!("client DLL loaded: {}", path);
            }
            Err(e) => {
                log::error!("client DLL load failed: {e}");
            }
        }
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    pub fn load_client_dll(&mut self, _path: &str) {}

    /// Client finished **init** (`SVC::InitDone`) — listen (channel) or UDP.
    pub(crate) fn client_in_play(&self) -> bool {
        matches!(
            self.client_engine.as_ref().map(|c| c.join_phase),
            Some(ClientSessionPhase::Play)
        )
    }

    /// Full client ECS + stream + render resource flush (disconnect / closehost / reconnect).
    #[cfg(any(feature = "client", feature = "full"))]
    fn flush_client_scene(&mut self) {
        if let Some(client) = self.client_engine.as_mut() {
            client.flush_client_scene(&mut self.mesh_registry);
            deerpunk_engine_client::draw_material_resolve::invalidate_all_draw_material_handles(
                client,
            );
            deerpunk_engine_client::draw_material_resolve::resolve_all_draw_materials(
                client,
                &self.resources,
            );
            if let Some(ref dll) = client.dll {
                unsafe {
                    dll.scene_flushed();
                }
            }
        }
    }

    /// Сервер готов — host подключает клиента как `connect localhost`, но через Channel.
    pub fn on_server_activated(&mut self) {
        log::info!("[host] server activated");
        #[cfg(any(feature = "client", feature = "full"))]
        self.connect_to_server_inprocess();
    }

    /// То же, что консольный `connect localhost`, без UDP (S2C/C2S = [`Channel`]).
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn connect_to_server_inprocess(&mut self) {
        if self.client_engine.is_none() {
            log::warn!("[net] connect inprocess: no client host");
            return;
        }
        if !self.listen_server_enabled() {
            log::warn!("[net] connect inprocess: server not running");
            return;
        }
        if self.client_net_active() {
            log::warn!("[net] connect inprocess: already UDP client");
            return;
        }
        self.client_engine.as_mut().unwrap().join_phase = ClientSessionPhase::Init;
        deerpunk_ui::set_client_in_game(false);
        self.flush_client_scene();
        let name = self.gameinfo.window_title().to_string();
        log::info!("[net] connect inprocess (server thread)");
        if let Some(st) = &self.server_thread {
            st.send(server_thread::ServerThreadCmd::ConnectLocal(name));
        }
        self.drain_loopback_from_server();
    }

    pub fn set_server_enabled(&mut self, on: bool) {
        #[cfg(any(feature = "client", feature = "full"))]
        if self.client_engine.is_some() {
            if !on {
                if let Some(client) = self.client_engine.as_mut() {
                    client.join_phase = ClientSessionPhase::Idle;
                }
                deerpunk_ui::set_client_in_game(false);
                self.flush_client_scene();
            }
            if on {
            } else {
                if self.client_net_active() {
                    self.disconnect_all_net();
                }
            }
            if let Some(st) = &self.server_thread {
                st.send(server_thread::ServerThreadCmd::SetEnabled(on));
            }
        }
    }

    /// Local listen client joins the running server (channel hop to the sim
    /// thread, which also runs join init + catchup).
    pub fn connect_local_client(&mut self) {
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            log::info!("[players] connect_local skipped (no client feature)");
        }
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let name = self.gameinfo.window_title().to_string();
            if let Some(st) = &self.server_thread {
                st.send(server_thread::ServerThreadCmd::ConnectLocal(name));
            }
        }
    }

    pub fn disconnect_local_client(&mut self) {
        if let Some(c) = self.client_engine.as_mut() {
            c.join_phase = ClientSessionPhase::Idle;
        }
        #[cfg(any(feature = "client", feature = "full"))]
        deerpunk_ui::set_client_in_game(false);
    }

    /// Legacy FFI — kept for ABI compatibility. The albedo atlas is now
    /// uploaded by the gamelib via `renderer.upload_texture_raw`; this just
    /// flushes lightmaps on the server world (dedicated tools) and returns
    /// the last gamelib-uploaded slot (always 0 in the new path).
    #[cfg(any(feature = "client", feature = "full"))]
    fn flush_pending_albedos(&mut self) -> u32 {
        let world_ptr: *const deerpunk_engine_common::world::World =
            if let Some(client) = self.client_engine.as_ref() {
                &client.world
            } else {
                return 0;
            };
        let _ = unsafe { self.flush_pending_albedos_world(&*world_ptr) };
        self.client_engine
            .as_mut()
            .map(|c| c.last_atlas_slot)
            .unwrap_or(0)
    }

    /// Legacy hook (no-op). Albedo/lightmap upload is via material URIs in gamelib.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn flush_pending_albedos_world(
        &mut self,
        _world: &deerpunk_engine_common::world::World,
    ) -> u32 {
        // Lightmaps and albedo atlases are uploaded via material `_Lightmap` / `_BaseMap` URIs
        // (`MaterialStore::attach_lightmaps_from_asset`, gamelib BSP loader). Kept for call sites.
        0
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn flush_pending_albedos(&mut self) -> u32 {
        0
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn drain_ui_commands(&mut self) {
        for line in deerpunk_ui::drain_commands() {
            log::info!("[ui] cmd: {line}");
            self.cmd.add_command(line);
        }
        self.cmd.frame();
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn drain_ui_commands(&mut self) {}

    /// Consume sim-thread → main messages: gamelib command names get a
    /// main-console PROXY that forwards the typed line back over `ExecCmd`
    /// (the real handler runs on the sim thread — old `main_cmd_proxy`
    /// semantics); the DLL-started message runs configs + starts the
    /// inspector. FIFO ordering guarantees proxies register before
    /// `finish_server_dll_startup` execs configs/stuffcmds.
    fn drain_server_to_host(&mut self) {
        let msgs = match self.server_thread.as_ref() {
            Some(st) => st.server_to_host_rx().try_iter().collect::<Vec<_>>(),
            None => return,
        };
        for msg in msgs {
            match msg {
                server_ctx::ServerToMain::RegisterProxyCommand(name) => {
                    let Some(st) = self.server_thread.as_ref() else {
                        continue;
                    };
                    let tx = st.cmd_sender().clone();
                    let cmd_name = name.clone();
                    self.cmd.register(
                        crate::cmd::CommandRegistration::new(name.clone()).execute(
                            move |ctx| {
                                let line = if ctx.args.is_empty() {
                                    cmd_name.clone()
                                } else {
                                    format!("{} {}", cmd_name, ctx.args.join(" "))
                                };
                                log::info!("[host] main → server thread: {line}");
                                let _ = tx.send(
                                    server_thread::ServerThreadCmd::ExecCmd(line),
                                );
                            },
                        ),
                    );
                    log::info!("[host] main-console proxy registered: '{name}'");
                }
                server_ctx::ServerToMain::ServerDllStarted => {
                    log::info!("[host] server gamelib started — exec configs + inspector");
                    self.finish_server_dll_startup();
                }
            }
        }
    }

    pub fn frame(&mut self, delta_time: f32) {
        {
            let _g = system_profiler::scope("frame.logic");
            let _g = system_profiler::scope("host.cmd");
            self.rebind_cmd_refs();
            stdin_cmd::drain(&self.cmd);
            self.drain_ui_commands();
            self.drain_server_to_host();
            self.time_secs += delta_time.max(0.0);
            #[cfg(any(feature = "client", feature = "full"))]
            self.drain_pending_demo_cmds();
        }

        // Console input before gamelib so open console eats keys for freecam/etc.
        // Nuklear draw runs later in [`Self::frame_ui`].
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("host.console");
            if let Some(client) = self.client_engine.as_mut() {
                client.console.tick_input(&self.cvar, &mut client.input);
                let cons_open = console::Console::visible(&self.cvar);
                client.set_console_open(cons_open);
                binds::global().tick(&client.input, &self.cmd, cons_open);
            }
        }

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("host.net");
            self.tick_net_client(delta_time);
        }

        {
            let _g = system_profiler::scope("host.replicate");
            #[cfg(any(feature = "client", feature = "full"))]
            if self.client_engine.as_mut().unwrap().demo_player.is_some() {
                self.tick_demo_playback(delta_time);
            } else {
                self.sync_client_world();
                self.demo_record_view_frame();
            }
            #[cfg(not(any(feature = "client", feature = "full")))]
            self.sync_client_world();
        }

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("client.dll");
            if let Some(client) = self.client_engine.as_mut() {
                client.clear_game_debug();
                if let Some(ref dll) = client.dll {
                    unsafe {
                        dll.frame(delta_time);
                    }
                }
            }
        }

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("client.ecs");
            if let Some(client) = self.client_engine.as_mut() {
                client.tick_world(delta_time);
            }
        }

        // Inspector WS: client world served synchronously, server world from
        // the sim-thread share.
        {
            let _g = system_profiler::scope("client.inspector");
            self.poll_server_inspector();
        }

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("client.post_frame");
            if let Some(client) = self.client_engine.as_mut() {
                if let Some(ref dll) = client.dll {
                    unsafe {
                        dll.post_frame(delta_time);
                    }
                }
            }
        }
    }

    /// Nuklear overlays — call from the renderer during `host_ui_begin_frame` … `host_ui_end_frame`.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn frame_ui(&mut self) {
        if let Some(client) = self.client_engine.as_ref() {
            client.gamelib_ui_build_frame(0.0);
        }
        if !deerpunk_engine_renderer::HOST_UI_ENABLED {
            return;
        }
        let show_fps = self.cvar.get_usize_or("cl_showfps", 0) != 0
            || self.cvar.get_usize_or("r_showfps", 0) != 0;
        let show_pos = self.cvar.get_usize_or("cl_showpos", 0) as i32;
        let net_show = self.cvar.get_usize_or("net_show", 0) as i32;
        let view = self.local_client_entity().and_then(|pawn| {
            self.client_engine.as_ref().and_then(|c| {
                deerpunk_engine_client::camera_util::read_local_view_stamp(&c.world, pawn)
            })
        });
        let local_client_ent = self.local_client_entity();
        let net_hud = if net_show > 0 {
            Some(self.net_hud_info(net_show))
        } else {
            None
        };
        let r_speeds = self.cvar.get_usize_or("r_speeds", 0) as i32;
        let render_hud = if r_speeds > 0 {
            Some(self.render_hud_info(r_speeds))
        } else {
            None
        };

        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        client.console.tick_draw(&self.cvar, &self.cmd, &self.cvar);
        let (view_yaw, view_pitch) = view.map(|v| (v.yaw, v.pitch)).unwrap_or((0.0, 0.0));
        deerpunk_engine_client::infobox::tick(
            show_fps,
            show_pos,
            &client.world,
            local_client_ent,
            view_yaw,
            view_pitch,
            net_hud.as_ref(),
            render_hud.as_deref(),
        );

        let r_showalloc = self.cvar.get_usize_or("r_showalloc", 0) as i32;
        if r_showalloc > 0 {
            deerpunk_engine_client::mesh_alloc_debug::tick(r_showalloc, client.renderer());
        }
    }

    /// Mirrored client-world entity for the local player (if any).
    #[cfg(any(feature = "client", feature = "full"))]
    fn local_client_entity(&self) -> Option<u64> {
        let client = self.client_engine.as_ref()?;
        client.rep.view_entity()
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn client_set_view_net_id(&mut self, net_id: u16) {
        self.client_engine
            .as_mut()
            .unwrap()
            .rep
            .set_view_net_id(net_id);
    }

    /// Send a registered UserMessage (`slot = None` → broadcast remotes).
    pub fn um_send(&mut self, name: &str, data: &[u8], slot: Option<u32>) -> Result<(), String> {
        let Some(msg_id) = self.um.id_of(name) else {
            return Err(format!("user message '{name}' not registered"));
        };
        let rpc = UserMessageRpc::new(msg_id, data.to_vec());
        let pkt = rpc.to_packet();
        match slot {
            None => self.deliver_s2c_broadcast(&[pkt]),
            Some(s) => self.deliver_s2c_to_slot(s, &[pkt]),
        }
        Ok(())
    }

    fn dispatch_user_message(&mut self, rpc: &UserMessageRpc) {
        self.um.invoke_hooks(rpc);
    }

    fn dispatch_user_message_packet(&mut self, packet: &[u8]) {
        let Some(payload) = packet.strip_prefix(&[SVC::UserMessage as u8]) else {
            return;
        };
        let Some(rpc) = UserMessageRpc::decode(payload) else {
            log::warn!("[um] bad UserMessage packet");
            return;
        };
        self.dispatch_user_message(&rpc);
    }

    fn local_client_in_play(&self) -> bool {
        self.client_in_play()
    }

    /// S2C to one slot: UDP or in-process [`Channel`].
    fn deliver_s2c_to_slot(&mut self, slot: u32, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        // Main host: local channel only; UDP delivery happens on the sim
        // thread (facade deliver path).
        let via = format!("s2c slot={slot} local");
        for p in packets {
            net_packet_log::log_s2c_out(&self.cvar, &via, p);
        }
        for p in packets {
            self.loopback.send_server_packet(p.clone());
        }
    }

    /// Play-phase replication to all clients in **play** (UDP + local channel).
    pub(crate) fn deliver_s2c_broadcast(&mut self, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        for p in packets {
            net_packet_log::log_s2c_out(&self.cvar, "s2c broadcast", p);
        }
        if self.local_client_in_play() {
            for p in packets {
                self.loopback.send_server_packet(p.clone());
            }
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn net_hud_info(&self, level: i32) -> deerpunk_engine_client::infobox::NetHudInfo {
        let st = if self.client_net_active() {
            self.client_engine.as_ref().unwrap().network.hud_stats()
        } else {
            deerpunk_engine_server::net::NetHudStats {
                role: "server",
                connected: self.listen_server_enabled(),
                ..Default::default()
            }
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "net: {}{}  in {:.0} B/s  out {:.0} B/s",
            st.role,
            if st.connected { "" } else { " (down)" },
            st.bytes_in_per_sec,
            st.bytes_out_per_sec
        ));
        if st.role == "client" || st.remote_clients > 0 {
            lines.push(format!("rtt: {:.0} ms", st.rtt_ms));
        }
        let _ = st;
        let _ = level;
        deerpunk_engine_client::infobox::NetHudInfo { lines }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn render_hud_info(&self, level: i32) -> Vec<String> {
        let Some(client) = self.client_engine.as_ref() else {
            return Vec::new();
        };
        let renderer = client.renderer();
        let p = renderer.frame_profile();
        let registry = &client.shader_registry;
        let mut lines = Vec::new();

        // GPU + VRAM.
        lines.push(renderer.adapter_label());
        let budget_mb = self.cvar.get_f32_or("r_vram_budget_mb", 1024.0);
        lines.push(renderer.vram_progress_line((budget_mb as u64).max(1) * 1024 * 1024));

        let s = &p.speeds_hud;
        lines.push(format!(
            "dip {}  shaders {}  tris {}",
            s.dip_count, s.shader_bind, s.frame_total_tris
        ));
        lines.push(format!(
            "cmds pass {} bind {} vb {} ib {} indirect {}",
            s.render_passes, s.bind_groups, s.vertex_binds, s.index_binds, s.indirect_draws
        ));
        lines.push(format!(
            "vis inst {}  occl {}/{}  lights w{} d{} s{}",
            s.visible_instances,
            s.occluded_objects,
            s.visible_occluders,
            s.visible_world_lights,
            s.visible_dyn_lights,
            s.visible_sun_lights
        ));

        if level >= 2 {
            lines.push(renderer.vram_breakdown_line());
            lines.push(format!(
                "render {:.2} ms (extract {:.2} + encode {:.2})",
                p.extract_ms + p.total_encode_ms,
                p.extract_ms,
                p.total_encode_ms
            ));
            lines.extend(deerpunk_engine_renderer::frame::format_graph_speeds_lines(
                p,
            ));
        }
        if level >= 3 {
            let eg = &p.extract_gpu;
            lines.push(format!(
                "extract sync {:.2} (skip {}) body {:.2} cull {:.2} post {:.2} mat {:.2} draws {}+{}",
                eg.sync_world_ms,
                eg.sync_world_unchanged,
                eg.extractor_ms,
                eg.cull_serial_ms,
                eg.post_ms,
                eg.resolve_ms,
                eg.mesh_draws,
                eg.skinned_draws,
            ));
            lines.push(format!(
                "{}x{}  mesh {} idx~{}  spr {} hud {}  lights {}",
                p.width,
                p.height,
                p.mesh_draws,
                p.mesh_index_count,
                p.sprite_instances,
                p.hud_instances,
                p.point_lights
            ));
            lines.push(format!(
                "cull frustum {:.2} assemble {:.2} gather {:.2} ms",
                p.prep_cull_frustum_ms, p.prep_cull_assemble_ms, p.prep_cull_gather_ms,
            ));
            lines.push(format!(
                "uploads {} (cached {})  finish {:.2} submit {:.2} present {:.2}",
                p.mesh_gpu_uploads,
                p.mesh_gpu_cached,
                p.encoder_finish_ms,
                p.queue_submit_ms,
                p.after_present_ms
            ));
        }
        if level >= 4 {
            let m = &p.mesh_stats;
            lines.push(format!(
                "mesh: submitted={}  indirect={} cmds={} bind_batch={}  lm_arr={} albedo2d={}",
                m.draws_submitted,
                m.indirect_batches,
                m.indirect_commands,
                m.bind_batch_draws,
                m.lm_array_layout_draws,
                m.albedo_2d_layout_draws
            ));
            if m.lm_array_with_gpu_id_zero > 0 {
                lines.push(format!(
                    "  WARN lm_arr layout but lightmap_gpu=0: {} draws",
                    m.lm_array_with_gpu_id_zero
                ));
            }
            if m.albedo_white_slot_draws > 0 {
                lines.push(format!(
                    "  albedo white slot: {} draws",
                    m.albedo_white_slot_draws
                ));
            }
            lines.push(format!(
                "  sample: id={} lm_arr={} alb={} lm={} mat={} bsp={}",
                m.sample_shader_id,
                if m.sample_lm_array { 1 } else { 0 },
                m.sample_albedo_slot,
                m.sample_lightmap_gpu,
                m.sample_material_id,
                if m.sample_has_bsp_mesh { 1 } else { 0 }
            ));
            lines.push(format!("    {}", m.sample_shader_path));
            for bin in &m.shader_bins {
                let path = registry
                    .path_for_id(bin.shader_id)
                    .unwrap_or("?")
                    .to_string();
                lines.push(format!(
                    "  id={} lm_arr={} draws={} tris~{}  {}",
                    bin.shader_id,
                    if bin.lm_array_layout { 1 } else { 0 },
                    bin.draws,
                    bin.tris,
                    path
                ));
            }
        }
        lines
    }

    fn drain_loopback(&mut self) {
        // Host commands only on the main thread; client CLC packets drain on
        // the sim thread (facade tick).
        self.drain_loopback_host_commands();
    }

    fn drain_loopback_host_commands(&mut self) {
        for cmd in self.loopback.drain_host_commands() {
            match cmd {
                HostCommand::LoadMap(name) => {
                    log::info!("[loopback] LoadMap {name}");
                    let _ = self.request_listen_map(&name, -1);
                }
                HostCommand::StopServer => {
                    log::info!("[loopback] stop server");
                    if let Some(st) = &self.server_thread {
                        st.send(server_thread::ServerThreadCmd::SetEnabled(false));
                    }
                    self.set_server_enabled(false);
                    move_state::clear();
                }
                HostCommand::InprocessConnect => {
                    self.on_server_activated();
                }
            }
        }
    }

    /// Client → server CLC (UserCmd, ClientCommand, …).
    #[cfg(any(feature = "client", feature = "full"))]
    pub(crate) fn send_clc_packet(&mut self, packet: Vec<u8>) {
        let via = if self.client_net_active() {
            "c2s udp"
        } else {
            "c2s local"
        };
        crate::net_packet_log::log_clc_out(&self.cvar, via, &packet);
        if self.client_net_active() {
            if !self.client_in_play() {
                return;
            }
            self.client_engine
                .as_mut()
                .unwrap()
                .network
                .send_c2s(packet);
            return;
        }
        if !self.client_in_play() || !self.listen_server_enabled() {
            return;
        }
        self.loopback.send_client_packet(packet);
    }

    pub fn alloc_sound_id(&mut self) -> u32 {
        let id = self.next_client_sound_id;
        self.next_client_sound_id = self.next_client_sound_id.wrapping_add(1);
        if self.next_client_sound_id == 0 {
            self.next_client_sound_id = 1;
        }
        id
    }

    pub fn queue_sound_packet(&mut self, packet: Vec<u8>) {
        self.loopback.send_server_packet(packet);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn sync_server_sound_origins(&mut self) {
        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        let pending = client.server_loops_with_net_id();
        let t_id = client
            .world
            .component_init::<deerpunk_engine_common::components::TransformComponent>();
        let mut updates = Vec::new();
        for (sound_id, net_id) in pending {
            let Some(ent) = client.rep.client_of_net(net_id) else {
                continue;
            };
            let ptr = client.world.get_raw(ent, t_id)
                as *const deerpunk_engine_common::components::TransformComponent;
            if ptr.is_null() {
                continue;
            }
            updates.push((sound_id, unsafe { &*ptr }.position.to_array()));
        }
        for (sound_id, origin) in updates {
            client.set_server_loop_origin(sound_id, origin);
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn apply_sound_play_packet(&mut self, packet: &[u8]) {
        let Some((_cmd, payload)) = packet.split_first() else {
            return;
        };
        let Some(rpc) = SoundPlayRpc::decode(payload) else {
            log::warn!("[net] SoundPlay decode failed");
            return;
        };
        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        client.audio_apply_play(&rpc);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn apply_sound_stop_packet(&mut self, packet: &[u8]) {
        let Some((_cmd, payload)) = packet.split_first() else {
            return;
        };
        let Some(rpc) = SoundStopRpc::decode(payload) else {
            log::warn!("[net] SoundStop decode failed");
            return;
        };
        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        client.audio_apply_stop(rpc.sound_id);
    }

    /// GS-style: one match for every SVC the local client can see.
    #[cfg(any(feature = "client", feature = "full"))]
    fn parse_server_message(&mut self, packet: &[u8]) {
        net_packet_log::log_s2c_in(&self.cvar, "client", packet);
        let Some(&cmd) = packet.first() else {
            return;
        };
        let Some(svc) = SVC::from_u8(cmd) else {
            log::warn!("[cl] unknown SVC cmd={cmd}");
            return;
        };
        match svc {
            SVC::Nop => {}
            SVC::Bad => log::warn!("[cl] SVC::Bad"),
            SVC::SoundPlay => self.apply_sound_play_packet(packet),
            SVC::SoundStop => self.apply_sound_stop_packet(packet),
            SVC::SpawnPrefab
            | SVC::StreamIn
            | SVC::NetDelta
            | SVC::StreamUpdate
            | SVC::NetDestroy
            | SVC::StreamOut
            | SVC::PrefabDef => {
                let Some(client) = self.client_engine.as_mut() else {
                    return;
                };
                let gamedir = self.fs.gamedir().to_path_buf();
                let _ = client.apply_replication_packet(
                    &mut self.mesh_registry,
                    &gamedir,
                    &self.resources,
                    packet,
                );
            }
            SVC::UserMessage => self.dispatch_user_message_packet(packet),
            SVC::UserCmdAck => {
                let Some(rpc) = UserCmdAckRpc::decode_packet(packet) else {
                    log::warn!("[cl] bad UserCmdAck");
                    return;
                };
                let Some(client) = self.client_engine.as_mut() else {
                    return;
                };
                client.usercmd_channel.set_ack(rpc.ack_sequence);
            }
            SVC::InitDone => {
                self.client_engine.as_mut().unwrap().join_phase = ClientSessionPhase::Play;
                deerpunk_ui::set_client_in_game(true);
                log::info!("[net] init done → play");
            }
            SVC::ParticlesEmit => self.apply_particles_emit_packet(packet),
        }
    }

    /// Client post-net-apply dispatch: spawn the burst in the local sim.
    #[cfg(any(feature = "client", feature = "full"))]
    fn apply_particles_emit_packet(&mut self, packet: &[u8]) {
        let Some((_cmd, payload)) = packet.split_first() else {
            return;
        };
        let Some(rpc) = particles_net::ParticlesEmitRpc::decode(payload) else {
            log::warn!("[net] ParticlesEmit decode failed");
            return;
        };
        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        client.spawn_particles_burst(&rpc.effect, rpc.pos, rpc.vel, &rpc.payload);
    }

    /// Record (if active) then [`Self::parse_server_message`].
    #[cfg(any(feature = "client", feature = "full"))]
    fn apply_local_s2c(&mut self, packet: &[u8]) {
        if packet.is_empty() {
            return;
        }
        // Demo play feeds parse directly — never re-record.
        if self.client_engine.as_mut().unwrap().demo_player.is_none() {
            let t = self.time_secs;
            if let Some(w) = self.client_engine.as_mut().unwrap().demo_writer.as_mut() {
                if let Err(e) = w.write_svc(t, packet) {
                    log::warn!("[demo] write: {e}");
                }
            }
        }
        self.parse_server_message(packet);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn demo_record_view_frame(&mut self) {
        let Some(pawn) = self.local_client_entity() else {
            return;
        };
        let client = self.client_engine.as_ref().unwrap();
        let Some(view) =
            deerpunk_engine_client::camera_util::read_local_view_stamp(&client.world, pawn)
        else {
            return;
        };
        let t = self.time_secs;
        let dv = demo::DemoView {
            yaw: view.yaw,
            pitch: view.pitch,
            roll: view.roll,
            eye_height: view.eye_height,
            fov_y: view.fov_y,
        };
        let Some(w) = self.client_engine.as_mut().unwrap().demo_writer.as_mut() else {
            return;
        };
        if let Err(e) = w.write_view(t, dv) {
            log::warn!("[demo] view write: {e}");
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn tick_demo_playback(&mut self, dt: f32) {
        let packets = {
            let Some(player) = self.client_engine.as_mut().unwrap().demo_player.as_mut() else {
                return;
            };
            player.tick(dt)
        };
        for packet in &packets {
            self.parse_server_message(packet);
        }
        if let Some(client) = self.client_engine.as_ref() {
            let world = &client.world as *const deerpunk_engine_common::world::World;
            let s = unsafe { self.flush_pending_albedos_world(&*world) };
            if s != 0 {
                self.client_engine.as_mut().unwrap().last_atlas_slot = s;
            }
        }
        let pawn = self.local_client_entity();
        if let Some(client) = self.client_engine.as_mut() {
            if let (Some(v), Some(pawn)) =
                (client.demo_player.as_ref().and_then(|p| p.last_view), pawn)
            {
                use deerpunk_engine_client::client_engine::LocalViewStamp;
                deerpunk_engine_client::local_view::stamp_view_on_entity(
                    &client.world,
                    pawn,
                    LocalViewStamp {
                        yaw: v.yaw,
                        pitch: v.pitch,
                        roll: v.roll,
                        eye_height: v.eye_height,
                        fov_y: v.fov_y,
                    },
                );
            }
        }
        let finished = self
            .client_engine
            .as_ref()
            .and_then(|c| c.demo_player.as_ref())
            .map(|p| p.finished())
            .unwrap_or(false);
        if finished {
            println!("[demo] finished");
            self.demo_stop();
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    /// Apply client view stamp to the local mirrored pawn (angles + eye + fov).
    #[cfg(any(feature = "client", feature = "full"))]
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn run(&mut self) {
        if self.client_engine.is_none() {
            return;
        }

        deerpunk_engine_common::world::install_panic_hook(
            &self.client_engine.as_ref().unwrap().world,
        );

        self.client_engine.as_mut().unwrap().init_extractor();

        let mut last_time = std::time::Instant::now();
        loop {
            system_profiler::begin_frame();
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_time).as_secs_f32();
            last_time = now;

            {
                let _g = system_profiler::scope("frame.bootstrap");
                stdin_cmd::drain(&self.cmd);
                self.drain_ui_commands();
                self.drain_loopback();

                let cons_open = self.cvar.get_bool_or("cl_console_show", false);
                if let Some(ce) = self.client_engine.as_mut() {
                    let ui_open = ce.ui_controller.any_visible();
                    ce.set_text_input_active(cons_open || ui_open);
                }

                self.client_engine
                    .as_mut()
                    .unwrap()
                    .set_audio_volume(self.cvar.get_f32_or("s_volume", 1.0));

                let packets = self.loopback.receive_server_packets();
                for packet in packets {
                    self.apply_local_s2c(&packet);
                }
            }

            if self.cmd.quit_requested() {
                system_profiler::end_frame();
                break;
            }

            {
                let _g = system_profiler::scope("frame.poll");
                if !self.client_engine.as_mut().unwrap().poll_events() {
                    system_profiler::end_frame();
                    break;
                }
            }

            let render_pipeline = self.cvar.get_usize_or("cl_render_thread", 0);
            if let Some(client) = self.client_engine.as_mut() {
                client.set_render_pipeline(render_pipeline);
            }
            let skip_gpu = self
                .client_engine
                .as_ref()
                .is_some_and(|client| client.should_skip_gpu());
            if skip_gpu {
                let _g = system_profiler::scope("frame.occluded");
                if let Some(client) = self.client_engine.as_mut() {
                    client.discard_pending_present();
                }
            } else if render_pipeline >= 1 {
                {
                    let _g = system_profiler::scope("frame.render_wait");
                    if let Some(client) = self.client_engine.as_mut() {
                        client.pipelined_submit_wait();
                    }
                }
                if let Some(client) = self.client_engine.as_mut() {
                    let _g = system_profiler::scope("frame.render_profile");
                    let profile = client.renderer().frame_profile();
                    deerpunk_engine_client::render_gpu_log::log_frame(&self.cvar, profile);
                    system_profiler::ingest_render_profile(profile);
                }
            } else if let Some(client) = self.client_engine.as_mut() {
                client.set_render_pipeline(0);
            }

            self.frame(dt);

            if skip_gpu {
                std::thread::sleep(std::time::Duration::from_millis(16));
                system_profiler::end_frame();
                continue;
            }

            {
                let _g = system_profiler::scope("frame.render_setup");
                let client = self.client_engine.as_mut().unwrap();
                deerpunk_engine_client::render_sync::apply_renderer_cvars(&self.cvar, client);
                client.advance_time(dt);
            }
            {
                let _g = system_profiler::scope("host.sound_origins");
                self.sync_server_sound_origins();
            }
            {
                let _g = system_profiler::scope("client.pre_render");
                if let Some(ref dll) = self.client_engine.as_mut().unwrap().dll {
                    unsafe {
                        dll.pre_render(dt);
                    }
                }
            }
            if self.cvar.get_usize_or("cl_predict", 1) != 0 {
                let smooth = self.cvar.get_f32_or("cl_smoothtime", 0.12);
                let nosmooth = self.cvar.get_usize_or("cl_nosmooth", 0) != 0;
                let view_smooth = self.cvar.get_f32_or("cl_viewsmooth", 18.0);
                {
                    let _g = system_profiler::scope("client.local_sim");
                    if let Some(client) = self.client_engine.as_mut() {
                        client
                            .local_sim
                            .tick_display(dt, smooth, nosmooth, view_smooth);
                    }
                }
                if let Some(ent) = self.local_client_entity() {
                    let _g = system_profiler::scope("client.stamp_simorg");
                    self.client_engine
                        .as_mut()
                        .unwrap()
                        .stamp_simorg_on_entity(ent);
                }
            }

            let view_ent = self.client_engine.as_mut().unwrap().rep.view_entity();
            let early_cull = self.cvar.get_usize_or("cl_extract_early_cull", 1) != 0;
            let draw_world = self.cvar.get_usize_or("r_drawworld", 1) != 0;
            let mut scene = {
                let _g = system_profiler::scope("frame.extract");
                let extract_start = std::time::Instant::now();
                let client = self.client_engine.as_mut().unwrap();
                let registry = &mut self.mesh_registry;
                let mut scene = client.extract_scene_view(
                    registry,
                    &self.resources,
                    view_ent,
                    early_cull,
                    draw_world,
                );
                scene.cull = deerpunk_engine_client::render_sync::frustum_cull_options(&self.cvar);
                let extract_ms = extract_start.elapsed().as_secs_f32() * 1000.0;
                self.client_engine
                    .as_mut()
                    .unwrap()
                    .renderer_mut()
                    .note_extract_ms(extract_ms);
                scene
            };
            if let Some(client) = self.client_engine.as_mut() {
                let _g = system_profiler::scope("frame.prepare");
                client.run_prepare(&mut scene);
                let geometry = &mut scene.geometry as *mut _;
                client.begin_mesh_submit(geometry);
                client.world.run_stage("renderer.prepare", dt);
                client.finish_mesh_submit(&mut scene);
            }

            {
                let _g = system_profiler::scope("frame.debug_draw");
                if let Some(client) = self.client_engine.as_ref() {
                    let null_phys = deerpunk_engine_common::physics::NullPhysics;
                    let mut draw = deerpunk_engine_client::render_sync::build_debug_draw(
                        &self.cvar,
                        &client.world,
                        &client.world,
                        &null_phys,
                        &self.mesh_registry,
                    );
                    if let Some(client) = self.client_engine.as_mut() {
                        draw.primitives.extend(client.take_game_debug().primitives);
                        *client.debug_draw() = draw;
                    }
                }
            }

            let host_ptr = self as *mut Self as *mut std::ffi::c_void;
            let rendered = {
                let _g = system_profiler::scope("frame.render");
                let ui_fps = self.cvar.get_f32_or("r_ui_fps", 30.0) as i32;
                self.client_engine.as_mut().unwrap().set_ui_fps(ui_fps);
                self.client_engine.as_mut().unwrap().tick_inner(
                    Some(&scene),
                    Some(host_frame_ui_trampoline),
                    host_ptr,
                    dt,
                    false,
                )
            };
            if render_pipeline == 0 {
                if let Some(client) = self.client_engine.as_ref() {
                    let _g = system_profiler::scope("frame.render_profile");
                    let profile = client.renderer().frame_profile();
                    deerpunk_engine_client::render_gpu_log::log_frame(&self.cvar, profile);
                    system_profiler::ingest_render_profile(profile);
                }
            }
            {
                let _g = system_profiler::scope("host.input_reset");
                self.client_engine
                    .as_mut()
                    .unwrap()
                    .input
                    .reset_mouse_delta();
            }
            system_profiler::end_frame();

            if !rendered || self.cmd.quit_requested() {
                break;
            }
        }

        self.shutdown();
        #[cfg(feature = "tracy")]
        deerpunk_engine_renderer::tracy_shutdown();
    }

    /// Persist configs and tear down DLLs. Safe to call once at exit.
    pub fn shutdown(&mut self) {
        config_io::write_all(&self.cvar, &self.fs);

        #[cfg(any(feature = "client", feature = "full"))]
        {
            if let Some(mut st) = self.server_thread.take() {
                log::info!("[host] stopping server thread…");
                st.shutdown();
                log::info!("[host] server thread stopped");
            }
            if let Some(ref dll) = self.client_engine.as_mut().unwrap().dll {
                unsafe {
                    dll.shutdown();
                }
            }
            self.client_engine.as_mut().unwrap().dll = None;
            // Documents → context → Rml::Shutdown before window/renderer die.
            if let Some(client) = self.client_engine.as_mut() {
                client.shutdown_render_pipeline();
                client.shutdown_ui();
            }
        }
        self.cmd.shutdown();
        self.cvar.shutdown();
        self.fs.shutdown();
        log::info!("[host] shutdown OK");
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}
