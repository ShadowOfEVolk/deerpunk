#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
pub type Entity = u64;

pub mod gamelib_abi;
pub use gamelib_abi::{ScheduleSystemFn, WorldQueryCallback};

pub mod binds;
pub mod cmd;
pub mod cmd_completion;
pub mod config_io;
pub mod console;
pub mod cvar;
mod client_sync;
#[cfg(any(feature = "client", feature = "full"))]
mod demo_console;
mod engine_methoddb;
mod net_console;
mod net_tick;
mod server_thread;
mod main_cmd_proxy;
pub mod fs;
pub mod gameinfo;
pub mod inspector;
mod mesh_components;
mod mesh_upload;
pub mod move_state;
mod physics_trace;
pub mod schedule;
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
use deerpunk_engine_common::network::protocol::{CLC, SVC};
use deerpunk_engine_common::network::decode_client_command_payload;
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};
use deerpunk_engine_common::network::user_message::UserMessageRpc;
use deerpunk_engine_common::network::{UserCmd, UserCmdAckRpc};
use deerpunk_engine_common::{LightstyleTable, MAX_LIGHTSTYLES};
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
// Foundation: cvar/cmd/fs/resources/mesh_registry/inspector/time.
// Server:     [`ServerEngine`] — sim + session (players, DLL, rep, LAN host).
// Client:     [`client_engine`] — render/input + session (DLL, console, rep, demo).
// Glue:       `loopback` (in-process CLC/SVC), `net` (UDP net socket + renet), `um`.

pub struct Host {
    /// Present only on the dedicated server thread (`Host::new_server_instance`).
    pub server_engine: Option<ServerEngine>,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<ClientEngine>,
    /// Shared listen-server wire queues (thread-safe).
    pub loopback: Arc<NetLoopback>,
    #[cfg(any(feature = "client", feature = "full"))]
    pub(crate) server_thread: Option<server_thread::ServerThread>,
    /// Shared with the listen-server simulation thread ([`Arc`] — table is internally locked).
    pub cvar: Arc<cvar::CvarSystem>,
    pub cmd: cmd::CmdSystem,
    pub fs: fs::FileSystem,
    pub gameinfo: gameinfo::GameInfo,
    pub resources: deerpunk_engine_common::resource::ResourceManager,
    /// Shared mesh handles for server spawn + client draw (not «client-only»).
    pub mesh_registry: deerpunk_engine_common::resource::MeshRegistry,
    /// Name → id map for custom mesh allocators registered via
    /// `v1:mesh:register_allocator`. Id `0` is reserved for the built-in
    /// slab allocator and is never stored here.
    pub mesh_allocator_names: std::collections::HashMap<String, u32>,
    pub um: user_messages::UserMessageRegistry,
    pub inspector: inspector::Inspector,
    pub(crate) time_secs: f32,
    /// Sound ids when [`server_engine`] is absent (client-only host).
    next_client_sound_id: u32,
    /// Listen-server: main reads this after [`apply_set_server_enabled`] on the sim thread.
    listen_enabled_sink: Option<Arc<std::sync::atomic::AtomicBool>>,
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
        if self.server_engine.is_some() {
            self.server_mut().network.disconnect();
        } else if let Some(st) = &self.server_thread {
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
        #[cfg(any(feature = "client", feature = "full"))]
        let client_engine = Some(ClientEngine::new(&resources));
        let loopback = Arc::new(NetLoopback::new());
        Self {
            server_engine: None,
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine,
            loopback,
            #[cfg(any(feature = "client", feature = "full"))]
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
            time_secs: 0.0,
            next_client_sound_id: 1,
            listen_enabled_sink: None,
        }
    }

    pub(crate) fn set_listen_enabled_sink(&mut self, sink: Arc<std::sync::atomic::AtomicBool>) {
        self.listen_enabled_sink = Some(sink);
    }

    /// Simulation thread [`Host`] only (listen-client main thread has `server_engine == None`).
    #[inline]
    #[track_caller]
    pub(crate) fn server_mut(&mut self) -> &mut ServerEngine {
        self.server_engine
            .as_mut()
            .expect("server_engine only on server thread Host")
    }

    #[inline]
    #[track_caller]
    pub(crate) fn server(&self) -> &ServerEngine {
        self.server_engine
            .as_ref()
            .expect("server_engine only on server thread Host")
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn listen_server_listening(&self) -> bool {
        self.server_engine
            .as_ref()
            .map(|e| e.network.is_listening())
            .unwrap_or_else(|| self.listen_server_enabled())
    }

    /// Server-thread `Host` (no client, full server sim).
    pub(crate) fn new_server_instance(
        loopback: Arc<NetLoopback>,
        gamedir: std::path::PathBuf,
        shared_cvar: Arc<cvar::CvarSystem>,
    ) -> Self {
        let mut resources = deerpunk_engine_common::resource::ResourceManager::new();
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
        Self {
            server_engine: Some(ServerEngine::new()),
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine: None,
            loopback,
            #[cfg(any(feature = "client", feature = "full"))]
            server_thread: None,
            cvar: shared_cvar,
            cmd: cmd::CmdSystem::new(),
            fs: fs::FileSystem::new(),
            gameinfo: gameinfo::GameInfo::from_gamedir(&gamedir),
            resources,
            mesh_registry: deerpunk_engine_common::resource::MeshRegistry::new(),
            mesh_allocator_names: std::collections::HashMap::new(),
            um: user_messages::UserMessageRegistry::default(),
            inspector: inspector::Inspector::new(),
            time_secs: 0.0,
            next_client_sound_id: 1,
            listen_enabled_sink: None,
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn listen_server_enabled(&self) -> bool {
        self.server_thread
            .as_ref()
            .map(|s| s.is_enabled())
            .unwrap_or(false)
    }

    pub(crate) fn server_initialize(&mut self, gamedir: &std::path::Path) {
        self.rebind_cmd_refs();
        self.fs.initialize();
        self.fs.set_gamedir(gamedir);
        deerpunk_engine_common::resource::register_default_loaders(&self.resources, &gamedir);
        self.cvar.ensure_initialized();
        self.cmd.initialize();
        binds::Binds::register_commands(&mut self.cmd);
        move_state::register_commands(&mut self.cmd);
        net_console::register_commands(&mut self.cmd);
        self.register_internal_components();
        if let Err(e) = self.server_mut().initialize(gamedir) {
            log::error!("[host] server_engine.initialize: {e}");
        }
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
    }

    pub(crate) fn load_server_dll_on_thread(&mut self, path: &str) {
        let ctx_ptr = self as *mut Host as *mut u8;
        let db = self.method_db();
        let resolver = db.into_resolver();
        match ServerDll::load(path) {
            Ok(server) => {
                unsafe {
                    server.init(ctx_ptr, resolver);
                }
                self.server_mut().dll = Some(server);
                log::info!("server DLL loaded (server thread): {}", path);
            }
            Err(e) => {
                log::error!("server DLL load failed: {e}");
            }
        }
    }

    pub(crate) fn finish_server_dll_startup(&mut self) {
        self.exec_game_rc();
        self.start_inspector();
    }

    /// Main-thread listen host: queue world load on the simulation thread (not `ExecCmd map`).
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
        if let Err(e) = st.cmd_sender().send(server_thread::ServerThreadCmd::StartMap {
            world: world.to_string(),
            seed,
        }) {
            log::error!("[host] start_listen_map send failed: {e}");
            return -1;
        }
        0
    }

    pub(crate) fn apply_gamelib_start_map(&mut self, world: &str, seed: i64) {
        let Some(dll) = self.server_mut().dll.as_ref() else {
            log::error!("[host] StartMap '{world}': server DLL not loaded");
            return;
        };
        unsafe {
            dll.start_map(world, seed);
        }
    }

    pub(crate) fn server_thread_tick(&mut self, dt: f32, enabled_flag: &std::sync::atomic::AtomicBool) {
        self.drain_loopback_client_packets();
        self.tick_net_server(dt);
        let on = self.server().enabled;
        enabled_flag.store(on, std::sync::atomic::Ordering::Relaxed);
        if !on {
            return;
        }
        let skin = self.cvar.get_f32_or("ph_capsule_skin", 0.5);
        self.server_mut().physics.set_capsule_skin(skin);
        if let Some(ref dll) = self.server_mut().dll {
            unsafe {
                dll.frame(dt);
            }
        }
        let _ = self.server_mut().frame(dt);
        let packets = self.server_mut().replication_packets();
        if !packets.is_empty() {
            self.deliver_s2c_broadcast(&packets);
        }
        self.deliver_usercmd_acks();
        self.poll_server_inspector();
        self.time_secs += dt;
    }

    fn poll_server_inspector(&mut self) {
        if !self.inspector.is_running() {
            return;
        }
        let time_secs = self.time_secs;
        let resources = &self.resources;
        let Some(engine) = self.server_engine.as_mut() else {
            return;
        };
        let ctx = inspector::InspectorRpcCtx {
            world: &engine.world,
            resources,
            time_secs,
            players: &engine.players,
            server_net: &engine.network,
            server_rep: &engine.rep,
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine: None,
        };
        self.inspector.poll(ctx);
    }

    pub(crate) fn server_shutdown(&mut self) {
        if let Some(engine) = self.server_engine.as_mut() {
            if let Some(dll) = engine.dll.as_ref() {
                unsafe {
                    dll.shutdown();
                }
            }
            engine.dll = None;
            engine.set_enabled(false);
        }
        if let Some(mut eng) = self.server_engine.take() {
            eng.shutdown();
        }
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

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let gamedir_path = gamedir.as_ref().to_path_buf();
            let server_thread = server_thread::ServerThread::spawn(
                self.loopback.clone(),
                gamedir_path,
                self.cvar.clone(),
            );
            main_cmd_proxy::attach(server_thread.cmd_sender().clone());
            self.server_thread = Some(server_thread);
            log::info!("[host] server simulation thread spawned");
        }

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
            }
            self.rebind_cmd_refs();
        }

        // Descriptors early; WS starts after gamelibs + `.rc` / stuffcmds (see `start_inspector`).
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
        stdin_cmd::start();

        log::info!("Engine initialized");
    }

    /// Start inspector WS using current cvars (after configs / stuffcmds).
    pub fn start_inspector(&mut self) {
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
            w.component_init::<PendingLightmap>();
            w.component_init::<LightmapId>();
            w.component_init::<MapKv>();
            w.component_init::<Collider>();
        };
        if let Some(engine) = self.server_engine.as_ref() {
            register(&engine.world);
        }
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            register(&client.world);
        }
    }

    pub fn method_db(&self) -> engine_api::engine_api::MethodDB {
        engine_api::engine_api::MethodDB::new()
    }

    pub fn load_server_dll(&mut self, path: &str) {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(st) = &self.server_thread {
            st.load_dll(path.to_string());
            return;
        }
        log::error!("[host] load_server_dll: no server thread");
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
        #[cfg(any(feature = "client", feature = "full"))]
        if self.client_engine.is_none() {
            log::warn!("[net] connect inprocess: no client host (sim thread?)");
            return;
        }
        #[cfg(any(feature = "client", feature = "full"))]
        let server_on = self
            .server_engine
            .as_ref()
            .map(|e| e.enabled)
            .unwrap_or_else(|| self.listen_server_enabled());
        #[cfg(not(any(feature = "client", feature = "full")))]
        let server_on = self.server().enabled;
        if !server_on {
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
        if let Some(st) = &self.server_thread {
            log::info!("[net] connect inprocess (server thread)");
            st.send(server_thread::ServerThreadCmd::ConnectLocal(name));
        } else {
            self.connect_local_client();
            let slot = self.server_mut().players.local_slot().unwrap_or(0);
            log::info!("[net] connect inprocess slot={slot} (channel)");
            self.finish_client_join(&[slot]);
        }
        self.drain_loopback_from_server();
    }

    /// `player_connected` + catchup + join init (UDP remote или in-process).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    fn finish_client_join(&mut self, slots: &[u32]) {
        if slots.is_empty() {
            return;
        }
        if self.server_engine.is_none() {
            log::error!("[net] finish_client_join on client-only host");
            return;
        }
        if !self.server().enabled {
            return;
        }
        for &slot in slots {
            if let Some(ref dll) = self.server_mut().dll {
                unsafe {
                    dll.player_connected(slot);
                }
            }
        }
        for &slot in slots {
            self.send_join_init(slot);
        }
        let engine = self.server_mut();
        let catchup = engine.rep.tick(&mut engine.world);
        self.deliver_s2c_broadcast(&catchup);
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
                if let Some(client) = self.client_engine.as_mut() {
                    client.last_ls_patterns = vec![String::new(); MAX_LIGHTSTYLES];
                    client.lightstyles = LightstyleTable::hl_defaults();
                }
            } else {
                if self.client_net_active() {
                    self.disconnect_all_net();
                }
                if let Some(client) = self.client_engine.as_mut() {
                    client.last_ls_patterns = vec![String::new(); MAX_LIGHTSTYLES];
                    client.lightstyles = LightstyleTable::hl_defaults();
                }
            }
            if let Some(st) = &self.server_thread {
                st.send(server_thread::ServerThreadCmd::SetEnabled(on));
                return;
            }
        }
        if self.server_engine.is_some() {
            self.apply_set_server_enabled(on);
        }
    }

    /// Server-thread only (listen / dedicated).
    pub(crate) fn apply_set_server_enabled(&mut self, on: bool) {
        let was = self.server().enabled;
        self.server_mut().set_enabled(on);
        if !on {
            self.server_mut().slots_play.clear();
        }
        if on {
            if !self.server_mut().network.is_listening() {
                let port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
                let addr = format!("0.0.0.0:{port}")
                    .parse()
                    .unwrap_or_else(|_| ([0, 0, 0, 0], net::DEFAULT_PORT).into());
                match connect_auth::load_or_create_private_key(self.fs.gamedir()) {
                    Ok(key) => {
                        if let Err(e) = self.server_mut().network.listen(addr, key) {
                            log::error!("[net] auto-listen failed: {e}");
                        } else {
                            self.start_lan_advertiser(port);
                        }
                    }
                    Err(e) => log::error!("[net] netcode.key: {e}"),
                }
            }
        } else {
            if was && self.server_mut().network.is_listening() {
                self.server_mut().network.disconnect();
            }
            self.server_mut().lan_advertiser = None;
            self.server_mut().lan_map.clear();
        }
        if let Some(sink) = &self.listen_enabled_sink {
            sink.store(self.server().enabled, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn start_lan_advertiser(&mut self, port: u16) {
        match lan::LanAdvertiser::new(port) {
            Ok(adv) => {
                self.server_mut().lan_advertiser = Some(adv);
                log::info!("[lan] advertising on port {port}");
            }
            Err(e) => {
                self.server_mut().lan_advertiser = None;
                log::warn!("[lan] advertiser: {e}");
            }
        }
    }

    fn lan_advertise_state(&self) -> lan::LanAdvertiseState {
        let hostname = self
            .cvar
            .get_string("hostname")
            .unwrap_or_else(|| "DarkDemo".into());
        let default_port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
        if let Some(sv) = self.server_engine.as_ref() {
            let port = sv.network.listen_port().unwrap_or(default_port);
            let map = if sv.lan_map.is_empty() {
                "?".into()
            } else {
                sv.lan_map.clone()
            };
            let players = sv.players.list().len().min(255) as u8;
            return lan::LanAdvertiseState {
                hostname,
                map,
                port,
                players,
                max_players: 16,
            };
        }
        lan::LanAdvertiseState {
            hostname,
            map: "?".into(),
            port: default_port,
            players: if self.listen_server_enabled() { 1 } else { 0 },
            max_players: 16,
        }
    }

    fn tick_lan(&mut self) {
        if self.server_engine.as_ref().is_some_and(|e| e.lan_advertiser.is_some()) {
            let state = self.lan_advertise_state();
            if let Some(adv) = self.server_engine.as_mut().unwrap().lan_advertiser.as_mut() {
                adv.tick(&state);
            }
        }

        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            client.lan_browser.poll();
            let mut list = client.lan_browser.servers();
            if self.listen_server_listening() {
                let st = self.lan_advertise_state();
                let local = lan::LanServerInfo {
                    addr: ([127, 0, 0, 1], st.port).into(),
                    hostname: st.hostname,
                    map: st.map,
                    port: st.port,
                    players: st.players,
                    max_players: st.max_players,
                };
                if !list
                    .iter()
                    .any(|s| s.port == local.port && s.addr.ip().is_loopback())
                {
                    list.insert(0, local);
                }
            }
            lan::set_servers_json(&list);
            deerpunk_ui::set_lan_servers_json(lan::servers_json());
        }
    }

    /// Local listen client joins the running server (PlayerConnection, then gamelib hook).
    pub fn connect_local_client(&mut self) {
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            log::info!("[players] connect_local skipped (no client feature)");
            return;
        }
        #[cfg(any(feature = "client", feature = "full"))]
        {
            if self.client_engine.is_none() {
                return;
            }
            let name = self.gameinfo.window_title().to_string();
            if self.server_engine.is_some() {
                if self.server_mut().players.connect_local(name).is_none() {
                    return;
                }
            } else if let Some(st) = &self.server_thread {
                st.send(server_thread::ServerThreadCmd::ConnectLocal(name));
            }
        }
    }

    pub fn disconnect_local_client(&mut self) {
        if self.server_engine.is_none() {
            if let Some(c) = self.client_engine.as_mut() {
                c.join_phase = ClientSessionPhase::Idle;
            }
            #[cfg(any(feature = "client", feature = "full"))]
            deerpunk_ui::set_client_in_game(false);
            return;
        }
        if let Some(conn) = self.server_mut().players.disconnect_local() {
            if let Some(ref dll) = self.server_mut().dll {
                unsafe {
                    dll.player_disconnected(conn.slot);
                }
            }
        }
        self.server_mut().players.clear();
        self.server_mut().slots_play.clear();
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
            if let Some(engine) = self.server_engine.as_ref() {
                &engine.world
            } else if let Some(client) = self.client_engine.as_ref() {
                &client.world
            } else {
                return 0;
            };
        let _ = unsafe { self.flush_pending_albedos_world(&*world_ptr) };
        self.client_engine.as_mut().map(|c| c.last_atlas_slot).unwrap_or(0)
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

    pub fn frame(&mut self, delta_time: f32) {
        {
            let _g = system_profiler::scope("frame.logic");
            let _g = system_profiler::scope("host.cmd");
            self.rebind_cmd_refs();
            stdin_cmd::drain(&self.cmd);
            self.drain_ui_commands();
            main_cmd_proxy::drain_pending_registrations(&mut self.cmd);
            self.time_secs += delta_time.max(0.0);
            #[cfg(any(feature = "client", feature = "full"))]
            self.drain_pending_demo_cmds();
        }

        // Console input before gamelib so open console eats keys for freecam/etc.
        // ImGui draw runs later in [`Self::frame_ui`] (inside NewFrame).
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("host.console");
            if let Some(client) = self.client_engine.as_mut() {
                client
                    .console
                    .tick_input(&self.cvar, &mut client.input);
                let cons_open = console::Console::visible(&self.cvar);
                client.set_console_open(cons_open);
                binds::global().tick(&client.input, &self.cmd, cons_open);
            }
        }

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("client.dll");
            if let Some(client) = self.client_engine.as_mut() {
                client.clear_game_debug();
                if let Some(ref dll) = client.dll {
                    unsafe {
                        dll.frame(delta_time);
                        dll.post_frame(delta_time);
                    }
                }
            }
        }

        {
            let _g = system_profiler::scope("host.net");
            #[cfg(any(feature = "client", feature = "full"))]
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

        // After SVC/demo apply so LightStyle UM hits GPU same frame.
        #[cfg(any(feature = "client", feature = "full"))]
        if self.client_in_play()
            || self.client_engine.as_mut().unwrap().demo_player.is_some()
            || (self.client_net_active() && self.client_engine.as_mut().unwrap().join_phase != ClientSessionPhase::Idle)
        {
            let _g = system_profiler::scope("host.lightstyles");
            self.tick_lightstyles();
        }
        // Inspector WS is polled on the server simulation thread (`poll_server_inspector`).
    }

    /// ImGui overlays — call only between renderer `imgui_begin_frame` / end.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn frame_ui(&mut self) {
        let show_fps = self.cvar.get_usize_or("cl_showfps", 0) != 0
            || self.cvar.get_usize_or("r_showfps", 0) != 0;
        let show_pos = self.cvar.get_usize_or("cl_showpos", 0) as i32;
        let net_show = self.cvar.get_usize_or("net_show", 0) as i32;
        let view = self.local_client_entity().and_then(|pawn| {
            self.client_engine.as_ref().and_then(|c| {
                deerpunk_engine_client::view_camera::read_local_view_stamp(&c.world, pawn)
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
        client
            .console
            .tick_draw(&self.cvar, &self.cmd, &self.cvar);
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

        // Client gamelib ImGui overlays (host-owned `imgui::Context`).
        if let Some(ref dll) = client.dll {
            let imgui_ctx = client.renderer().imgui_context_ptr();
            unsafe {
                dll.imgui_frame(imgui_ctx);
            }
        }
    }

    /// Mirrored client-world entity for the local player (if any).
    #[cfg(any(feature = "client", feature = "full"))]
    fn local_client_entity(&self) -> Option<u64> {
        let client = self.client_engine.as_ref()?;
        if let Some(e) = client.rep.view_entity() {
            return Some(e);
        }
        if self.server_engine.is_none() {
            return None;
        }
        let slot = self.server().players.local_slot()?;
        let sid = self.server().players.get(slot)?.entity?;
        let net_id = self.server().rep.net_id_of(sid)?;
        client.rep.client_of_net(net_id)
    }

    /// `NetworkEntity::net_id` for a player slot's pawn (after replication tick).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub fn player_net_id(&self, slot: u32) -> Option<u16> {
        let engine = self.server_engine.as_ref()?;
        let sid = engine.players.get(slot)?.entity?;
        engine.rep.net_id_of(sid)
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn client_set_view_net_id(&mut self, net_id: u16) {
        self.client_engine.as_mut().unwrap().rep.set_view_net_id(net_id);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn client_set_lightstyle(&mut self, style: u8, pattern: &str) {
        self.client_engine.as_mut().unwrap().lightstyles.set_pattern(style as usize, pattern);
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
        if self.server_engine.is_none() {
            return self.client_in_play();
        }
        let Some(slot) = self.server().players.local_slot() else {
            return false;
        };
        self.server().slots_play.accepts_usercmd(slot)
    }

    /// S2C to one slot: UDP or in-process [`Channel`].
    fn deliver_s2c_to_slot(&mut self, slot: u32, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        let is_local = if let Some(engine) = self.server_engine.as_ref() {
            engine.players.get(slot).map(|p| p.local).unwrap_or(false)
        } else {
            #[cfg(any(feature = "client", feature = "full"))]
            {
                self.client_engine.is_some()
            }
            #[cfg(not(any(feature = "client", feature = "full")))]
            {
                false
            }
        };
        let via = if is_local {
            format!("s2c slot={slot} local")
        } else {
            format!("s2c slot={slot} udp")
        };
        for p in packets {
            net_packet_log::log_s2c_out(&self.cvar, &via, p);
        }
        if is_local {
            for p in packets {
                self.loopback.send_server_packet(p.clone());
            }
        } else if let Some(engine) = self.server_engine.as_mut() {
            engine.network.send_s2c_to_slot(slot, packets);
        }
    }

    /// Play-phase replication to all clients in **play** (UDP + local channel).
    fn deliver_s2c_broadcast(&mut self, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        for p in packets {
            net_packet_log::log_s2c_out(&self.cvar, "s2c broadcast", p);
        }
        if let Some(engine) = self.server_engine.as_mut() {
            engine.network.broadcast_s2c(packets);
        }
        if self.local_client_in_play() {
            for p in packets {
                self.loopback.send_server_packet(p.clone());
            }
        }
    }

    fn deliver_usercmd_acks(&mut self) {
        let tick = {
            let engine = self.server_mut();
            engine.sim_tick = engine.sim_tick.wrapping_add(1);
            engine.sim_tick
        };
        let slots = self.server().players.slots();
        let mut pending: Vec<(u32, Vec<u8>)> = Vec::new();
        {
            let engine = self.server_mut();
            for slot in slots {
                let Some(p) = engine.players.get(slot) else {
                    continue;
                };
                let seq = p.usercmd.sequence;
                if seq <= p.cmd_ack_sent {
                    continue;
                }
                if let Some(p) = engine.players.get_mut(slot) {
                    p.cmd_ack_sent = seq;
                }
                let rpc = UserCmdAckRpc {
                    ack_sequence: seq,
                    server_tick: tick,
                };
                pending.push((slot, rpc.encode_packet()));
            }
        }
        for (slot, packet) in pending {
            self.deliver_s2c_to_slot(slot, &[packet]);
        }
    }

    /// Join **init** bundle for one slot (then **play** via `InitDone`).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    fn send_join_init(&mut self, slot: u32) {
        let gamedir = self.fs.gamedir().to_path_buf();
        let engine = self.server_mut();
        let defs = engine
            .rep
            .prefab_definition_packets(&gamedir, &engine.world);
        let snap = engine.rep.snapshot(&engine.world);
        let def_count = defs.len();
        let snap_count = snap.len();
        self.deliver_s2c_to_slot(slot, &defs);
        if let Some(ref dll) = self.server_mut().dll {
            unsafe {
                dll.join_init(slot, 0);
            }
        }
        self.deliver_s2c_to_slot(slot, &snap);
        if let Some(ref dll) = self.server_mut().dll {
            unsafe {
                dll.join_init(slot, 1);
            }
        }
        self.deliver_s2c_to_slot(slot, &[vec![SVC::InitDone as u8]]);
        self.server_mut().slots_play.mark_ready(slot);
        log::info!(
            "[net] init slot={slot}: {def_count} prefab defs, {snap_count} snap pkts → InitDone"
        );
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn net_hud_info(&self, level: i32) -> deerpunk_engine_client::infobox::NetHudInfo {
        let st = if self.client_net_active() {
            self.client_engine.as_ref().unwrap().network.hud_stats()
        } else if let Some(engine) = self.server_engine.as_ref() {
            engine.network.hud_stats()
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
        if level >= 2 {
            if let Some(engine) = self.server_engine.as_ref() {
                lines.push(format!(
                    "loss: {:.1}%  remotes: {}  players: {}",
                    st.packet_loss * 100.0,
                    st.remote_clients,
                    engine.players.list().len()
                ));
                for p in engine.players.list() {
                    let ent = p
                        .entity
                        .map(|e| format!("{e:#x}"))
                        .unwrap_or_else(|| "-".into());
                    let kind = if p.local { "local" } else { "remote" };
                    lines.push(format!(
                        "  slot {} [{}] {} ent={}",
                        p.slot, kind, p.name, ent
                    ));
                }
                for (slot, addr) in engine.network.remote_slot_debug() {
                    lines.push(format!("  net slot {slot} ← {addr}"));
                }
            }
        }
        deerpunk_engine_client::infobox::NetHudInfo { lines }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn render_hud_info(&self, level: i32) -> Vec<String> {
        let Some(client) = self.client_engine.as_ref() else {
            return Vec::new();
        };
        let p = client.renderer().frame_profile();
        let registry = &client.shader_registry;
        let mut lines = Vec::new();
        let s = &p.speeds_hud;
        lines.push(format!("DIP count   {}", s.dip_count));
        lines.push(format!("Shader bind {}", s.shader_bind));
        lines.push(format!("Frame total tris {}", s.frame_total_tris));
        lines.push(format!("Visible world lights {}", s.visible_world_lights));
        lines.push(format!("Visible dyn lights {}", s.visible_dyn_lights));
        lines.push(format!("Visible sun lights {}", s.visible_sun_lights));
        lines.push(format!("Visible instances {}", s.visible_instances));
        lines.push(format!("Visible occluders {}", s.visible_occluders));
        lines.push(format!("Occluded objects {}", s.occluded_objects));

        let listed_encode = p.graph_nodes_cpu_ms();
        let encode_gap = (p.total_encode_ms - listed_encode).max(0.0);
        if level >= 2 {
            lines.push(format!(
                "render: {:.2} ms (extract {:.2} + encode {:.2})",
                p.extract_ms + p.total_encode_ms,
                p.extract_ms,
                p.total_encode_ms
            ));
        }
        if level >= 2 {
            lines.push(format!(
                "  graph listed {:.2} + other {:.2} | finish {:.2} queue_submit {:.2}",
                listed_encode, encode_gap, p.encoder_finish_ms, p.queue_submit_ms
            ));
            lines.push(format!(
                "  tick_inner: prep {:.2} (ui {:.2} cull {:.2} audio {:.2})  game_ui {:.2}  acquire {:.2}",
                p.prep_ms,
                p.prep_ui_ms,
                p.prep_cull_ms,
                p.prep_audio_ms,
                p.game_frame_ui_ms,
                p.surface_acquire_ms
            ));
            lines.push(format!(
                "  tail: imgui {:.2}  finish {:.2}  submit {:.2}  present {:.2}",
                p.after_imgui_ms, p.encoder_finish_ms, p.queue_submit_ms, p.after_present_ms
            ));
        }
        if level >= 2 {
            let specs = client.renderer().forward_graph_describe();
            lines.extend(deerpunk_engine_renderer::frame::format_graph_speeds_lines(
                p, &specs,
            ));
        }
        if level >= 3 {
            let eg = &p.extract_gpu;
            lines.push(format!(
                "  extract: body {:.2} serial {:.2} post {:.2} ms | mat {:.2} ms draws {}+{}",
                eg.extractor_ms,
                eg.cull_serial_ms,
                eg.post_ms,
                eg.resolve_ms,
                eg.mesh_draws,
                eg.skinned_draws,
            ));
            lines.push(format!(
                "  extract cache: rebuild {} xform {} hit {} frustum_skip {}",
                eg.extract_static_rebuild,
                eg.extract_xform_only,
                eg.extract_cache_hit,
                eg.extract_frustum_skip,
            ));
            lines.push(format!(
                "  {}x{}  mesh {} tris~{}  spr {} hud {} terr {} lights {}",
                p.width,
                p.height,
                p.mesh_draws,
                p.mesh_index_count,
                p.sprite_instances,
                p.hud_instances,
                p.terrain_draws,
                p.point_lights
            ));
            lines.push(format!(
                "  cull: frustum {:.2} assemble {:.2} gather {:.2} ms (wall {:.2})",
                p.prep_cull_frustum_ms,
                p.prep_cull_assemble_ms,
                p.prep_cull_gather_ms,
                p.prep_cull_ms,
            ));
            let m = &p.mesh_stats;
            if m.uniforms_ms > 0.0 || m.mesh_for_draw_ms > 0.0 || m.draw_encode_ms > 0.0 {
                lines.push(format!(
                    "  mesh node: uni {:.2} gpu_cache {:.2} draw {:.2} ms | up {} hit {}",
                    m.uniforms_ms,
                    m.mesh_for_draw_ms,
                    m.draw_encode_ms,
                    p.mesh_gpu_uploads,
                    p.mesh_gpu_cached
                ));
            }
            if p.debug_verts > 0 {
                lines.push(format!("  debug verts {}", p.debug_verts));
            }
        }
        if level >= 4 {
            let m = &p.mesh_stats;
            lines.push(format!(
                "mesh: submitted={}  lm_arr={} albedo2d={}",
                m.draws_submitted, m.lm_array_layout_draws, m.albedo_2d_layout_draws
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
        self.drain_loopback_host_commands();
        if self.server_engine.is_some() {
            self.drain_loopback_client_packets();
        }
    }

    fn drain_loopback_host_commands(&mut self) {
        #[cfg(any(feature = "client", feature = "full"))]
        if self.server_engine.is_some() && self.client_engine.is_none() {
            // Host commands (InprocessConnect, LoadMap, …) are for the main-thread client Host.
            return;
        }
        for cmd in self.loopback.drain_host_commands() {
            match cmd {
                HostCommand::LoadMap(name) => {
                    log::info!("[loopback] LoadMap {name}");
                    let _ = self.request_listen_map(&name, -1);
                }
                HostCommand::StopServer => {
                    log::info!("[loopback] stop server");
                    if self.server_engine.is_some() {
                        self.cmd.add_command("disconnect".into());
                        self.cmd.frame();
                        self.apply_set_server_enabled(false);
                        move_state::clear();
                    } else {
                        if let Some(st) = &self.server_thread {
                            st.send(server_thread::ServerThreadCmd::SetEnabled(false));
                        }
                        self.set_server_enabled(false);
                        move_state::clear();
                    }
                }
                HostCommand::InprocessConnect => {
                    self.on_server_activated();
                }
            }
        }
    }

    /// GS-style: one match for every CLC the server accepts.
    fn parse_client_message(&mut self, slot: u32, packet: &[u8]) {
        net_packet_log::log_clc_in(&self.cvar, slot, packet);
        let Some(&cmd) = packet.first() else {
            return;
        };
        let Some(clc) = CLC::from_u8(cmd) else {
            log::warn!("[sv] unknown CLC cmd={cmd} from slot={slot}");
            return;
        };
        match clc {
            CLC::Nop => {}
            CLC::Bad => log::warn!("[sv] CLC::Bad from slot={slot}"),
            CLC::UserCmd => {
                let engine = self.server_mut();
                if engine.enabled && !engine.slots_play.accepts_usercmd(slot) {
                    return;
                }
                let payload = &packet[1..];
                let Some(ucmd) = UserCmd::decode(payload) else {
                    log::warn!("[sv] bad UserCmd len={} from slot={slot}", payload.len());
                    return;
                };
                engine.players.set_usercmd(slot, ucmd);
            }
            CLC::ClientCommand => {
                let engine = self.server_mut();
                if engine.enabled && !engine.slots_play.accepts_usercmd(slot) {
                    return;
                }
                let payload = &packet[1..];
                let Some(line) = decode_client_command_payload(payload) else {
                    log::warn!("[sv] bad ClientCommand from slot={slot}");
                    return;
                };
                engine
                    .pending_client_commands
                    .push((slot, line.to_string()));
            }
        }
    }

    fn drain_loopback_client_packets(&mut self) {
        if self.server_engine.is_none() {
            return;
        }
        let slot = self.server().players.local_slot().unwrap_or(0);
        for packet in self.loopback.receive_client_packets() {
            self.parse_client_message(slot, &packet);
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
            self.client_engine.as_mut().unwrap().network.send_c2s(packet);
            return;
        }
        let server_on = self
            .server_thread
            .as_ref()
            .map(|s| s.is_enabled())
            .unwrap_or_else(|| self.server_engine.as_ref().map(|e| e.enabled).unwrap_or(false));
        if !self.client_in_play() || !server_on {
            return;
        }
        self.loopback.send_client_packet(packet);
    }

    pub fn alloc_sound_id(&mut self) -> u32 {
        if let Some(engine) = self.server_engine.as_mut() {
            let id = engine.next_sound_id;
            engine.next_sound_id = engine.next_sound_id.wrapping_add(1);
            if engine.next_sound_id == 0 {
                engine.next_sound_id = 1;
            }
            return id;
        }
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
        }
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
            deerpunk_engine_client::view_camera::read_local_view_stamp(&client.world, pawn)
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
            if let (Some(v), Some(pawn)) = (
                client.demo_player.as_ref().and_then(|p| p.last_view),
                pawn,
            ) {
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
    /// Evaluate client lightstyle table → GPU scales (patterns via gamelib UM hooks).
    #[cfg(any(feature = "client", feature = "full"))]
    fn tick_lightstyles(&mut self) {
        if let Some(client) = self.client_engine.as_mut() {
            client.tick_lightstyles(self.time_secs);
        }
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn tick_lightstyles(&mut self) {}

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn run(&mut self) {
        if self.client_engine.is_none() {
            return;
        }

        let world_ptr = self.client_engine.as_ref().unwrap().world.as_ptr();
        deerpunk_engine_common::world::install_panic_hook(world_ptr);

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
                main_cmd_proxy::drain_pending_registrations(&mut self.cmd);
                self.drain_loopback();

                let cons_open = self.cvar.get_bool_or("cl_console_show", false);
                self.client_engine
                    .as_mut()
                    .unwrap()
                    .set_text_input_active(cons_open);

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

            self.frame(dt);

            {
                let _g = system_profiler::scope("frame.render_setup");
                let client = self.client_engine.as_mut().unwrap();
                deerpunk_engine_client::render_sync::apply_renderer_cvars(&self.cvar, client);
                client.advance_time(dt);
            }
            self.sync_server_sound_origins();
            if let Some(ref dll) = self.client_engine.as_mut().unwrap().dll {
                unsafe {
                    dll.pre_render(dt);
                }
            }
            if self.cvar.get_usize_or("cl_predict", 1) != 0 {
                let smooth = self.cvar.get_f32_or("cl_smoothtime", 0.12);
                let nosmooth = self.cvar.get_usize_or("cl_nosmooth", 0) != 0;
                let view_smooth = self.cvar.get_f32_or("cl_viewsmooth", 18.0);
                if let Some(client) = self.client_engine.as_mut() {
                    client
                        .local_sim
                        .tick_display(dt, smooth, nosmooth, view_smooth);
                }
                if let Some(ent) = self.local_client_entity() {
                    self.client_engine
                        .as_mut()
                        .unwrap()
                        .stamp_simorg_on_entity(ent);
                }
            }

            let view_ent = self.client_engine.as_mut().unwrap().rep.view_entity();
            let early_cull = self.cvar.get_usize_or("cl_extract_early_cull", 1) != 0;
            let scene = {
                let _g = system_profiler::scope("frame.extract");
                let extract_start = std::time::Instant::now();
                let (client, registry) = self.split_client_registry();
                let mut scene = client.extract_scene_view(registry, view_ent, early_cull);
                scene.cull =
                    deerpunk_engine_client::render_sync::frustum_cull_options(&self.cvar);
                let extract_ms = extract_start.elapsed().as_secs_f32() * 1000.0;
                self.client_engine
                    .as_mut()
                    .unwrap()
                    .renderer_mut()
                    .note_extract_ms(extract_ms);
                scene
            };

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
                        draw.primitives
                            .extend(client.take_game_debug().primitives);
                        *client.debug_draw() = draw;
                    }
                }
            }

            let host_ptr = self as *mut Self as *mut std::ffi::c_void;
            let rendered = {
                let _g = system_profiler::scope("frame.render");
                self.client_engine.as_mut().unwrap().tick_inner(
                    Some(&scene),
                    Some(host_frame_ui_trampoline),
                    host_ptr,
                    false,
                )
            };
            if let Some(client) = self.client_engine.as_ref() {
                let profile = client.renderer().frame_profile();
                deerpunk_engine_client::render_gpu_log::log_frame(&self.cvar, profile);
                system_profiler::ingest_render_profile(profile);
            }
            system_profiler::end_frame();

            if !rendered || self.cmd.quit_requested() {
                break;
            }
            self.client_engine
                .as_mut()
                .unwrap()
                .input
                .reset_mouse_delta();
        }

        self.shutdown();
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

/// Resolve a BSP path relative to gamedir.
/// Accepts bare map name (`oldlifehome`), `maps/foo.bsp`, or absolute path.
fn resolve_bsp_path(gamedir: &std::path::Path, path: &str) -> std::path::PathBuf {
    let path = path.trim();
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if path.contains('/') || path.contains('\\') || path.to_ascii_lowercase().ends_with(".bsp") {
        return gamedir.join(path);
    }
    let name = path.trim_end_matches(".bsp");
    gamedir.join("maps").join(format!("{name}.bsp"))
}
