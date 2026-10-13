#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
use std::sync::atomic::{AtomicBool, Ordering};
pub type Entity = u64;

/// `v1:world:query_execute` gamelib callback.
pub type WorldQueryCallback =
    extern "C" fn(u64, *const *const std::ffi::c_void, usize, *mut std::ffi::c_void);

/// `v1:schedule:add_system` gamelib system fn (stub host schedule).
pub type ScheduleSystemFn = extern "C" fn(f32, *mut std::ffi::c_void);

static REQUEST_QUIT: AtomicBool = AtomicBool::new(false);
/// `s_play <event>` console commands queued for the client frame.
static S_PLAY_QUEUE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

enum PendingNetCmd {
    Listen(Option<u16>),
    Connect(String),
    NetDisconnect,
    LanRefresh,
    LanStop,
}
static PENDING_NET: std::sync::Mutex<Vec<PendingNetCmd>> = std::sync::Mutex::new(Vec::new());

#[cfg(any(feature = "client", feature = "full"))]
enum PendingDemoCmd {
    Record(String),
    Play(String),
    Stop,
}
#[cfg(any(feature = "client", feature = "full"))]
static PENDING_DEMO: std::sync::Mutex<Vec<PendingDemoCmd>> = std::sync::Mutex::new(Vec::new());

pub mod binds;
#[cfg(any(feature = "client", feature = "full"))]
pub mod phys_debug;
pub mod entity_debug;
pub mod config_io;
pub mod cmd;
mod engine_methoddb;
mod world_host_api;
mod spawn_prefab;
mod physics_trace;
pub mod console;
pub mod cvar;
pub mod fs;
pub mod gameinfo;
pub mod inspector;
pub mod move_state;
pub mod schedule;
pub mod stdin_cmd;
pub mod system_profiler;
pub mod players;
pub use schedule::Schedule;
pub use players::{PlayerConnection, PlayerConnections};

pub mod api;
pub use api::{ComponentId, EntityId};

pub mod server_engine;
pub use server_engine::ServerEngine;

pub mod engine_api;
pub use deerpunk_engine_common::world;

use deerpunk_engine_common::network::protocol::{CLC, SVC};
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};
use deerpunk_engine_common::{LightstyleTable, MAX_LIGHTSTYLES};
use deerpunk_engine_common::network::user_message::UserMessageRpc;
use deerpunk_engine_common::network::UserCmd;
use connection::{ClientSessionPhase, SlotPlayGate};
pub use crate::cmd::CmdCallback;
pub use engine_api::ui::*;

pub mod channel;
pub mod client_dll;
#[cfg(any(feature = "client", feature = "full"))]
pub mod demo;
pub mod connect_auth;
pub mod connection;
pub mod lan;
pub mod net;
pub mod udp_demux;
pub mod server_dll;
pub mod user_messages;
pub use channel::{Channel, ClientCmd};
pub use client_dll::{ClientDll, ClientInitData};
pub use server_dll::{ResolverFn, ServerDll, ServerInitData};

pub mod common {
    pub use deerpunk_engine_common::*;
    pub mod ipc {
        pub use deerpunk_engine_common::ipc::*;
    }
    pub mod frame_buffer {
        pub use deerpunk_engine_common::frame_buffer::*;
    }
}

#[cfg(feature = "dedicated-server")]
pub mod server {
    pub use deerpunk_engine_server::*;
}

#[cfg(feature = "client")]
pub mod client {
    pub use deerpunk_engine_client::*;
}

// ── Built-in CvarDefs ─────────────────────────
// flags: ARCHIVE → configs/config.cfg, RENDERINFO → configs/video.cfg

inventory::submit! {
    cvar::CvarDef { name: "r_texturehandle", default: "", desc: "Shared memory texture handle for editor frame output", flags: 0 }
}
inventory::submit! {
    cvar::CvarDef { name: "gamedir", default: "examples/deathmatch/gamedir", desc: "Game directory path", flags: 0 }
}
inventory::submit! {
    cvar::CvarDef {
        name: "gamename",
        default: "",
        desc: "Game folder name (from gamedir path / gameinfo). Used for {gamename}.rc",
        flags: 0,
    }
}
inventory::submit! {
    cvar::CvarDef { name: "ipc_debug", default: "0", desc: "Enable IPC debug logging (1=log all messages)", flags: 0 }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_inspector_port",
        default: "9000",
        desc: "WebSocket port for deerpunk-inspector (0 = disabled).",
        flags: cvar::ARCHIVE,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_inspector_host",
        default: "127.0.0.1",
        desc: "Bind address for inspector WS (e.g. 127.0.0.1 or 0.0.0.0).",
        flags: cvar::ARCHIVE,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "cl_console_show",
        default: "0",
        desc: "Show in-game ImGui console (1=visible). Toggle with ~.",
        flags: 0,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "cl_demoplaying",
        default: "0",
        desc: "1 while demo play is active (disables mlook).",
        flags: 0,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_fullbright",
        default: "0",
        desc: "Ignore lightmaps (1=albedo only, HL-style fullbright).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_lightmap",
        default: "0",
        desc: "Debug: draw lightmap only (1=on).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_phys_debug",
        default: "0",
        desc: "Wireframe world collider part under crosshair (0=off, 1=on).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_drawentities",
        default: "0",
        desc: "Draw client entities (0=off, 1=bbox, 2=eye lines, 3=both, 4=mesh AABB).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_cheats",
        default: "0",
        desc: "Allow cheat commands (noclip). 0=off, 1=on.",
        flags: 0,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_port",
        default: "28015",
        desc: "UDP listen port for multiplayer (renet).",
        flags: cvar::ARCHIVE,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "hostname",
        default: "DarkDemo",
        desc: "Server name shown in LAN browser.",
        flags: cvar::ARCHIVE,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "net_show",
        default: "0",
        desc: "Infobox net: 0=off, 1=role+rtt+B/s, 2=+loss+players.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "cl_showfps",
        default: "0",
        desc: "Show FPS in the client infobox (0=off, 1=on).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "cl_showpos",
        default: "0",
        desc: "Infobox pos/angles: 0=off, 1=pos+ang+view, 2=+speed, 3=+vel xyz+(speed).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_showfps",
        default: "0",
        desc: "Alias of cl_showfps (legacy).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "s_volume",
        default: "1",
        desc: "Master audio volume (FMOD bus:/ volume, 0..1).",
        flags: cvar::ARCHIVE,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "s_debug",
        default: "0",
        desc: "Audio debug output (1=on).",
        flags: 0,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_exposure",
        default: "1",
        desc: "Exposure for tonemap. -1 = auto-exposure, otherwise fixed multiplier. 1 = GoldSrc-style.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_tonemap",
        default: "-1",
        desc: "Tonemap mode: -1=passthrough (GoldSrc-style raw), 0=ACES, 1=Reinhard, 2=Hable.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_bloom",
        default: "1",
        desc: "Enable HDR bloom postfx (0/1).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_bloom_threshold",
        default: "0.7",
        desc: "Bloom luminance threshold.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_bloom_intensity",
        default: "1",
        desc: "Bloom add intensity.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_grain",
        default: "1",
        desc: "Enable film grain postfx (0/1).",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_grain_strength",
        default: "0.025",
        desc: "Film grain strength.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_grain_downscale",
        default: "3",
        desc: "Film grain noise tile scale.",
        flags: cvar::RENDERINFO,
    }
}

#[cfg(any(feature = "client", feature = "full"))]
extern "C" fn host_frame_ui_trampoline(ctx: *mut std::ffi::c_void) {
    let host = unsafe { &mut *(ctx as *mut Host) };
    host.frame_ui();
}

// ── Host ─────────────────────────────────────────────────────

/// Client-owned camera stamp (reapplied after world replicate).
#[cfg(any(feature = "client", feature = "full"))]
#[derive(Clone, Copy, Debug)]
struct LocalViewStamp {
    yaw: f32,
    pitch: f32,
    roll: f32,
    eye_height: f32,
    fov_y: f32,
}

pub struct Host {
    pub server_engine: ServerEngine,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<ClientEngine>,
    pub cvar: cvar::CvarSystem,
    pub cmd: cmd::CmdSystem,
    pub fs: fs::FileSystem,
    pub gameinfo: gameinfo::GameInfo,
    pub resources: deerpunk_engine_common::resource::ResourceManager,
    pub channel: Channel,
    pub players: players::PlayerConnections,
    server_dll: Option<ServerDll>,
    #[cfg(any(feature = "client", feature = "full"))]
    client_dll: Option<ClientDll>,
    #[cfg(any(feature = "client", feature = "full"))]
    console: console::Console,
    /// Server → client prefab replication (listen-server via Channel).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    server_rep: deerpunk_engine_server::network::ServerPrefabReplication,
    #[cfg(any(feature = "client", feature = "full"))]
    client_rep: deerpunk_engine_client::network::ClientPrefabReplication,
    /// UDP multiplayer (renet). Host and remote client.
    pub net: net::NetTransport,
    /// In-process / UDP client until `SVC::InitDone`.
    client_session_phase: ClientSessionPhase,
    /// Slots in **play** (after join init).
    slots_play: SlotPlayGate,
    /// LAN broadcast advertiser (hosting only).
    lan_advertiser: Option<lan::LanAdvertiser>,
    /// Menu LAN browser (connectionless on sv_port).
    lan_browser: lan::LanBrowser,
    /// Current map name for LAN info (set on place_bsp).
    lan_map: String,
    /// Server-assigned sound instance ids (`SoundPlay` / `SoundStop`).
    next_sound_id: u32,
    /// Named SVC UserMessage registry + client hooks.
    pub um: user_messages::UserMessageRegistry,
    /// Client lightstyle patterns — filled only via LightStyle SVC (incl. listen local).
    client_lightstyles: LightstyleTable,
    /// Last patterns broadcast via LightStyle UM (diff).
    last_ls_patterns: Vec<String>,
    /// Accumulated host time for lightstyle animation (`a`..`z`).
    time_secs: f32,
    /// Last client-authored view stamp (survives replicate overwrite).
    #[cfg(any(feature = "client", feature = "full"))]
    local_view: Option<LocalViewStamp>,
    #[cfg(any(feature = "client", feature = "full"))]
    demo_writer: Option<demo::DemoWriter>,
    #[cfg(any(feature = "client", feature = "full"))]
    demo_player: Option<demo::DemoPlayer>,
    pub inspector: inspector::Inspector,
    /// Last albedo atlas slot uploaded from the client world's
    /// `PendingAlbedoAtlas` (set by `sync_client_world` / demo / net flushes).
    /// Exposed to gamelib via `flush_pending_albedos` FFI so it can patch the
    /// `DrawMaterial`'s `_BaseMap` — the internal flushes consume the pending
    /// component and would otherwise lose the slot.
    #[cfg(any(feature = "client", feature = "full"))]
    last_atlas_slot: u32,
}

impl Host {
    pub fn new() -> Self {
        // Do not set_refs here: returning Self moves cvar/fs and would leave
        // dangling pointers inside cmd. Rebind in `rebind_cmd_refs` after place is stable.
        let mut resources = deerpunk_engine_common::resource::ResourceManager::new();
        // FileLoader must exist before client loads WGSL through the asset manager.
        resources.register_loader(Box::new(
            deerpunk_engine_common::resource::FileLoader,
        ));
        #[cfg(any(feature = "client", feature = "full"))]
        let client_engine = Some(ClientEngine::new(&mut resources));
        Self {
            server_engine: ServerEngine::new(),
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine,
            cvar: cvar::CvarSystem::new(),
            cmd: cmd::CmdSystem::new(),
            fs: fs::FileSystem::new(),
            gameinfo: gameinfo::GameInfo::from_gamedir(std::path::Path::new(".")),
            resources,
            channel: Channel::new(),
            players: players::PlayerConnections::default(),
            server_dll: None,
            #[cfg(any(feature = "client", feature = "full"))]
            client_dll: None,
            #[cfg(any(feature = "client", feature = "full"))]
            console: console::Console::new(),
            #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
            server_rep: deerpunk_engine_server::network::ServerPrefabReplication::new(),
            #[cfg(any(feature = "client", feature = "full"))]
            client_rep: deerpunk_engine_client::network::ClientPrefabReplication::new(),
            net: net::NetTransport::default(),
            client_session_phase: ClientSessionPhase::Idle,
            slots_play: SlotPlayGate::default(),
            lan_advertiser: None,
            lan_browser: lan::LanBrowser::default(),
            lan_map: String::new(),
            next_sound_id: 1,
            um: user_messages::UserMessageRegistry::default(),
            client_lightstyles: LightstyleTable::hl_defaults(),
            last_ls_patterns: vec![String::new(); MAX_LIGHTSTYLES],
            time_secs: 0.0,
            #[cfg(any(feature = "client", feature = "full"))]
            local_view: None,
            #[cfg(any(feature = "client", feature = "full"))]
            demo_writer: None,
            #[cfg(any(feature = "client", feature = "full"))]
            demo_player: None,
            inspector: inspector::Inspector::new(),
            #[cfg(any(feature = "client", feature = "full"))]
            last_atlas_slot: 0,
        }
    }

    /// CmdSystem holds raw pointers into Host fields — refresh after any move.
    fn rebind_cmd_refs(&mut self) {
        let cmd = &mut self.cmd;
        let cvar = &self.cvar;
        let fs = &self.fs;
        cmd.set_refs(cvar, fs);
    }

    pub fn initialize(&mut self, gamedir: impl AsRef<std::path::Path>) {
        self.rebind_cmd_refs();
        self.fs.initialize();
        self.fs.set_gamedir(&gamedir);
        #[cfg(any(feature = "client", feature = "full"))]
        deerpunk_ui::set_content_root(gamedir.as_ref());
        deerpunk_engine_common::resource::register_default_loaders(
            &mut self.resources,
            gamedir.as_ref(),
        );
        self.cvar.initialize();
        self.cmd.initialize();
        binds::Binds::register_commands(&mut self.cmd);
        move_state::register_commands(&mut self.cmd);
        self.cmd.register_command("quit", |_| {
            REQUEST_QUIT.store(true, Ordering::SeqCst);
        });
        self.register_net_commands();
        #[cfg(any(feature = "client", feature = "full"))]
        self.register_demo_commands();
        self.cmd.set_cmdline(std::env::args().collect());
        self.gameinfo = gameinfo::GameInfo::from_gamedir(gamedir.as_ref());
        self.cvar.set_or_create("gamename", &self.gameinfo.gamefolder);
        self.cvar.set_or_create("gamedir", &gamedir.as_ref().to_string_lossy());
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            let title = self.gameinfo.window_title().to_string();
            let icon = self.gameinfo.icon_path(gamedir.as_ref());
            client.apply_window_identity(&title, icon.as_deref());
        }
        // Configs + stuffcmds live in `{gamename}.rc`, exec'd after gamelibs.
        self.register_internal_components();

        if let Err(e) = self.server_engine.initialize(gamedir.as_ref()) {
            log::error!("[host] server_engine.initialize: {e}");
        }

        // UI runtime before gamelib so `v1:ui:*` works from server_init.
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let gamedir_ui = self.fs.resolve_ui_dir();
            if let Some(client) = self.client_engine.as_mut() {
                client.init_ui(gamedir_ui);
            }
            // Audio: FMOD backend, banks from `{gamedir}/sound`.
            let gamedir = gamedir.as_ref().to_path_buf();
            if let Some(client) = self.client_engine.as_mut() {
                client.set_audio_backend(
                    Box::new(deerpunk_engine_client_fmod::FmodBackend::default()),
                    &gamedir,
                );
            }
            self.cmd.register_command("s_play", |ctx| {
                let event = ctx.args.join(" ");
                if event.is_empty() {
                    log::info!("[s_play] usage: s_play <event_path> (e.g. event:/ambient/fan)");
                    return;
                }
                S_PLAY_QUEUE.lock().unwrap().push(event);
            });
        }

        // Descriptors early; WS starts after gamelibs + `.rc` / stuffcmds (see `start_inspector`).
        self.inspector.register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
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
        self.inspector.register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
            id: "systems".into(),
            label: "Systems".into(),
            group: deerpunk_inspector_protocol::CollectionGroup::Runtime,
            modes: vec![deerpunk_inspector_protocol::CollectionMode::List],
            id_field: "name".into(),
            nested_in: None,
        });
        #[cfg(any(feature = "client", feature = "full"))]
        self.inspector.register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
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
        self.inspector.register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
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
        self.inspector.register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
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
        self.inspector.sync_methoddb();
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
        register(&self.server_engine.world);
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            register(&client.world);
        }
    }

    pub fn method_db(&self) -> engine_api::engine_api::MethodDB {
        engine_api::engine_api::MethodDB::new()
    }

    pub fn load_server_dll(&mut self, path: &str) {
        let ctx_ptr = self as *mut Host as *mut u8;
        let db = self.method_db();
        let resolver = db.into_resolver();

        match ServerDll::load(path) {
            Ok(server) => {
                unsafe {
                    server.init(ctx_ptr, resolver);
                }
                self.server_dll = Some(server);
                log::info!("server DLL loaded: {}", path);
            }
            Err(e) => {
                log::error!("server DLL load failed: {}. Running without server gamelib.", e);
            }
        }

        // Xash: `{gamefolder}.rc` (exec configs…; stuffcmds). After gamelibs so cmds exist.
        self.exec_game_rc();

        // After gamelibs + startup cmds (autoexec may set sv_inspector_*).
        self.start_inspector();
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
                self.client_dll = Some(client);
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
        matches!(self.client_session_phase, ClientSessionPhase::Play)
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
        if !self.server_engine.enabled {
            log::warn!("[net] connect inprocess: server not running");
            return;
        }
        if self.net.is_client() {
            log::warn!("[net] connect inprocess: already UDP client");
            return;
        }
        self.client_session_phase = ClientSessionPhase::Init;
        deerpunk_ui::set_client_in_game(false);
        if let Some(client) = self.client_engine.as_mut() {
            self.client_rep.clear(&client.world);
        }
        self.server_rep.clear();
        self.connect_local_client();
        let Some(slot) = self.players.local_slot() else {
            return;
        };
        self.slots_play.remove(slot);
        log::info!("[net] connect inprocess slot={slot} (channel)");
        self.finish_client_join(&[slot]);
        self.drain_client_channel_s2c();
    }

    /// `player_connected` + catchup + join init (UDP remote или in-process).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    fn finish_client_join(&mut self, slots: &[u32]) {
        if slots.is_empty() || !self.server_engine.enabled {
            return;
        }
        for &slot in slots {
            if let Some(ref dll) = self.server_dll {
                unsafe {
                    dll.player_connected(slot);
                }
            }
        }
        let catchup = self.server_rep.tick(&self.server_engine.world);
        self.deliver_s2c_broadcast(&catchup);
        for &slot in slots {
            self.send_join_init(slot);
        }
    }

    pub fn set_server_enabled(&mut self, on: bool) {
        let was = self.server_engine.enabled;
        self.server_engine.set_enabled(on);
        if !on {
            self.slots_play.clear();
            self.client_session_phase = ClientSessionPhase::Idle;
            #[cfg(any(feature = "client", feature = "full"))]
            deerpunk_ui::set_client_in_game(false);
        }
        if on {
            if was {
                self.disconnect_local_client();
            }
            // Force full LightStyle UM resync; client scales come only via SVC.
            self.last_ls_patterns = vec![String::new(); MAX_LIGHTSTYLES];
            self.client_lightstyles = LightstyleTable::hl_defaults();
            // Auto-listen UDP so a second process can `connect`.
            if !self.net.is_hosting() && !self.net.is_client() {
                let port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
                let addr = format!("0.0.0.0:{port}")
                    .parse()
                    .unwrap_or_else(|_| ([0, 0, 0, 0], net::DEFAULT_PORT).into());
                match connect_auth::load_or_create_private_key(self.fs.gamedir()) {
                    Ok(key) => {
                        if let Err(e) = self.net.listen(addr, key) {
                            log::error!("[net] auto-listen failed: {e}");
                        } else {
                            self.start_lan_advertiser(port);
                        }
                    }
                    Err(e) => log::error!("[net] netcode.key: {e}"),
                }
            }
        } else {
            // Full closehost / disconnect — even if server was already off (e.g. pure client).
            if was {
                self.disconnect_local_client();
            }
            if self.net.is_hosting() || self.net.is_client() {
                self.net.disconnect();
            }
            self.lan_advertiser = None;
            self.lan_map.clear();
            self.last_ls_patterns = vec![String::new(); MAX_LIGHTSTYLES];
            self.client_lightstyles = LightstyleTable::hl_defaults();
        }
    }

    fn start_lan_advertiser(&mut self, port: u16) {
        match lan::LanAdvertiser::new(port) {
            Ok(adv) => {
                self.lan_advertiser = Some(adv);
                log::info!("[lan] advertising on port {port}");
            }
            Err(e) => {
                self.lan_advertiser = None;
                log::warn!("[lan] advertiser: {e}");
            }
        }
    }

    fn lan_advertise_state(&self) -> lan::LanAdvertiseState {
        let port = self
            .net
            .listen_port()
            .unwrap_or_else(|| self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16);
        let hostname = self.cvar.get_string("hostname").unwrap_or_else(|| "DarkDemo".into());
        let map = if self.lan_map.is_empty() {
            "?".into()
        } else {
            self.lan_map.clone()
        };
        let players = self.players.list().len().min(255) as u8;
        lan::LanAdvertiseState {
            hostname,
            map,
            port,
            players,
            max_players: 16,
        }
    }

    fn tick_lan(&mut self) {
        if self.lan_advertiser.is_some() {
            let state = self.lan_advertise_state();
            if let Some(adv) = self.lan_advertiser.as_mut() {
                adv.tick(&state);
            }
        }

        self.lan_browser.poll();

        // Local listen-server always visible in our own browser list.
        let mut list = self.lan_browser.servers();
        if self.net.is_hosting() {
            let st = self.lan_advertise_state();
            let local = lan::LanServerInfo {
                addr: ([127, 0, 0, 1], st.port).into(),
                hostname: st.hostname,
                map: st.map,
                port: st.port,
                players: st.players,
                max_players: st.max_players,
            };
            if !list.iter().any(|s| s.port == local.port && s.addr.ip().is_loopback()) {
                list.insert(0, local);
            }
        }
        lan::set_servers_json(&list);
        #[cfg(any(feature = "client", feature = "full"))]
        deerpunk_ui::set_lan_servers_json(lan::servers_json());
    }

    fn register_net_commands(&mut self) {
        self.cmd.register_command("listen", |ctx| {
            let port = ctx
                .args
                .first()
                .and_then(|s| s.parse::<u16>().ok());
            PENDING_NET.lock().unwrap().push(PendingNetCmd::Listen(port));
        });
        self.cmd.register_command("connect", |ctx| {
            let Some(addr) = ctx.args.first().copied() else {
                println!("Usage: connect <ip[:port]>  (default port {})", net::DEFAULT_PORT);
                return;
            };
            PENDING_NET
                .lock()
                .unwrap()
                .push(PendingNetCmd::Connect(addr.to_string()));
        });
        self.cmd.register_command("net_disconnect", |_| {
            PENDING_NET
                .lock()
                .unwrap()
                .push(PendingNetCmd::NetDisconnect);
        });
        self.cmd.register_command("lan_refresh", |_| {
            PENDING_NET.lock().unwrap().push(PendingNetCmd::LanRefresh);
        });
        self.cmd.register_command("lan_stop", |_| {
            PENDING_NET.lock().unwrap().push(PendingNetCmd::LanStop);
        });
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn register_demo_commands(&mut self) {
        self.cmd.register_command("record", |ctx| {
            let Some(name) = ctx.args.first().copied() else {
                println!("Usage: record <name>");
                return;
            };
            PENDING_DEMO
                .lock()
                .unwrap()
                .push(PendingDemoCmd::Record(name.to_string()));
        });
        self.cmd.register_command("playdemo", |ctx| {
            let Some(name) = ctx.args.first().copied() else {
                println!("Usage: playdemo <name>");
                return;
            };
            PENDING_DEMO
                .lock()
                .unwrap()
                .push(PendingDemoCmd::Play(name.to_string()));
        });
        // GS alias
        self.cmd.register_command("play", |ctx| {
            let Some(name) = ctx.args.first().copied() else {
                println!("Usage: play <name>  (demo)");
                return;
            };
            PENDING_DEMO
                .lock()
                .unwrap()
                .push(PendingDemoCmd::Play(name.to_string()));
        });
        self.cmd.register_command("stop", |_| {
            PENDING_DEMO.lock().unwrap().push(PendingDemoCmd::Stop);
        });
    }

    fn drain_pending_net_cmds(&mut self) {
        let cmds = std::mem::take(&mut *PENDING_NET.lock().unwrap());
        for cmd in cmds {
            match cmd {
                PendingNetCmd::Listen(port_opt) => {
                    let port = port_opt.unwrap_or_else(|| {
                        self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16
                    });
                    self.cvar.set_or_create("sv_port", &port.to_string());
                    let addr = format!("0.0.0.0:{port}")
                        .parse()
                        .unwrap_or_else(|_| ([0, 0, 0, 0], net::DEFAULT_PORT).into());
                    match connect_auth::load_or_create_private_key(self.fs.gamedir()) {
                        Ok(key) => match self.net.listen(addr, key) {
                            Ok(()) => {
                                println!("[net] listening on {addr} (secure)");
                                self.start_lan_advertiser(port);
                            }
                            Err(e) => println!("[net] listen failed: {e}"),
                        },
                        Err(e) => println!("[net] netcode.key: {e}"),
                    }
                }
                PendingNetCmd::Connect(addr_s) => {
                    match net::parse_addr(&addr_s, net::DEFAULT_PORT) {
                        Ok(addr) => {
                            let port = self
                                .net
                                .listen_port()
                                .unwrap_or_else(|| {
                                    self.cvar
                                        .get_f32_or("sv_port", net::DEFAULT_PORT as f32)
                                        as u16
                                });
                            #[cfg(any(feature = "client", feature = "full"))]
                            if self.server_engine.enabled
                                && self.net.is_hosting()
                                && net::is_connect_to_local_listen(addr, port)
                            {
                                println!("[net] connect {addr} → in-process (channel)");
                                self.connect_to_server_inprocess();
                                continue;
                            }
                            if self.server_engine.enabled {
                                self.set_server_enabled(false);
                            }
                            #[cfg(any(feature = "client", feature = "full"))]
                            {
                                if let Some(client) = self.client_engine.as_mut() {
                                    self.client_rep.clear(&client.world);
                                }
                            }
                            self.client_session_phase = ClientSessionPhase::Idle;
                            match self.net.connect(addr) {
                                Ok(()) => {
                                    println!("[net] connecting to {addr}… (auth)");
                                }
                                Err(e) => println!("[net] connect failed: {e}"),
                            }
                        }
                        Err(e) => println!("[net] connect: {e}"),
                    }
                }
                PendingNetCmd::NetDisconnect => {
                    self.net.disconnect();
                    self.client_session_phase = ClientSessionPhase::Idle;
                    self.slots_play.clear();
                    self.lan_advertiser = None;
                    #[cfg(any(feature = "client", feature = "full"))]
                    {
                        if let Some(client) = self.client_engine.as_mut() {
                            self.client_rep.clear(&client.world);
                        }
                        deerpunk_ui::set_client_in_game(false);
                    }
                    println!("[net] net_disconnect");
                }
                PendingNetCmd::LanRefresh => {
                    let port = self.cvar.get_f32_or("sv_port", net::DEFAULT_PORT as f32) as u16;
                    match self.lan_browser.ensure_listening(port) {
                        Ok(()) => {
                            self.lan_browser.send_query();
                            println!("[lan] refresh port={port}");
                        }
                        Err(e) => println!("[lan] refresh failed: {e}"),
                    }
                }
                PendingNetCmd::LanStop => {
                    self.lan_browser.stop();
                    lan::set_servers_json(&[]);
                    #[cfg(any(feature = "client", feature = "full"))]
                    deerpunk_ui::set_lan_servers_json("[]");
                    println!("[lan] browser stopped");
                }
            }
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
            if self.players.connect_local(name).is_none() {
                return;
            }
        }
    }

    pub fn disconnect_local_client(&mut self) {
        if let Some(conn) = self.players.disconnect_local() {
            if let Some(ref dll) = self.server_dll {
                unsafe {
                    dll.player_disconnected(conn.slot);
                }
            }
        }
        self.players.clear();
        self.slots_play.clear();
        self.client_session_phase = ClientSessionPhase::Idle;
        #[cfg(any(feature = "client", feature = "full"))]
        deerpunk_ui::set_client_in_game(false);
    }

    /// Upload pending albedo atlases on the server world (legacy path / dedicated tools).
    /// Returns the albedo atlas slot (0 if none).
    ///
    /// The client world's `PendingAlbedoAtlas` is flushed internally by
    /// `sync_client_world` / demo / net paths — those uploads happen on the
    /// host tick and the slot is stored in `last_atlas_slot`. This method
    /// returns that stored slot so the gamelib can patch a `DrawMaterial`'s
    /// `_BaseMap`. Also flushes the server world (legacy / dedicated tools).
    #[cfg(any(feature = "client", feature = "full"))]
    fn flush_pending_albedos(&mut self) -> u32 {
        // Flush server world too (legacy / dedicated tools path). For listen
        // servers this is usually a no-op (PendingAlbedoAtlas is on the client
        // world), but keep it for completeness.
        let world = &self.server_engine.world as *const deerpunk_engine_common::world::World;
        // SAFETY: server world is owned by self; client_engine borrow is separate.
        let server_slot = unsafe { self.flush_pending_albedos_world(&*world) };
        if server_slot != 0 {
            self.last_atlas_slot = server_slot;
        }
        self.last_atlas_slot
    }

    /// Upload [`PendingAlbedoAtlas`] / [`PendingLightmap`] from `world` into the client renderer.
    ///
    /// Returns the albedo atlas texture slot (0 if none uploaded this call).
    /// The slot is valid in the renderer's shared texture array — use it to
    /// build a `DrawMaterial` that references the atlas as `_BaseMap`.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn flush_pending_albedos_world(&mut self, world: &deerpunk_engine_common::world::World) -> u32 {
        use deerpunk_engine_common::components::{
            LightmapId, Material, PendingAlbedoAtlas, PendingLightmap,
        };

        let Some(client) = self.client_engine.as_mut() else {
            return 0;
        };
        let pending_id = world.component_init::<PendingAlbedoAtlas>();
        let pending_lm_id = world.component_init::<PendingLightmap>();
        let lm_id_comp = world.component_init::<LightmapId>();
        let mat_id = world.component_init::<Material>();

        let ents: Vec<u64> = world
            .inspect_entities()
            .into_iter()
            .filter(|e| {
                e.components.iter().any(|c| {
                    c.contains("PendingAlbedoAtlas") || c.contains("PendingLightmap")
                })
            })
            .map(|e| e.id)
            .collect();

        let mut last_atlas_slot = 0u32;

        for ent in ents {
            let mut uploaded_atlas = false;
            let ptr = world.get_raw(ent, pending_id) as *mut PendingAlbedoAtlas;
            if !ptr.is_null() {
                let pending = unsafe { std::ptr::read(ptr) };
                unsafe {
                    std::ptr::write(ptr, PendingAlbedoAtlas::default());
                }
                if let Some(img) = pending.image {
                    let tex = client
                        .renderer_mut()
                        .upload_texture(&img.pixels, img.width, img.height);
                    log::debug!(
                        "[host] albedo atlas ent={ent:#x} {}x{} -> slot={}",
                        img.width,
                        img.height,
                        tex.0
                    );
                    last_atlas_slot = tex.0;
                    let mat_ptr = world.get_raw(ent, mat_id) as *mut Material;
                    if !mat_ptr.is_null() {
                        unsafe {
                            (*mat_ptr).albedo = tex;
                        }
                    } else {
                        let mut mat = Material::default();
                        mat.color = [1.0, 1.0, 1.0, 1.0];
                        mat.roughness = 0.9;
                        mat.albedo = tex;
                        world.set(ent, mat_id, mat);
                    }
                    uploaded_atlas = true;
                }
                world.remove(ent, pending_id);
            }

            let lm_ptr = world.get_raw(ent, pending_lm_id) as *mut PendingLightmap;
            if !lm_ptr.is_null() {
                let pending_lm = unsafe { std::ptr::read(lm_ptr) };
                unsafe {
                    std::ptr::write(lm_ptr, PendingLightmap::default());
                }
                if let Some(atlas) = pending_lm.atlas {
                    let id = client.renderer_mut().upload_lightmap_layers(
                        atlas.width,
                        atlas.height,
                        [
                            &atlas.layers[0].pixels,
                            &atlas.layers[1].pixels,
                            &atlas.layers[2].pixels,
                            &atlas.layers[3].pixels,
                        ],
                    );
                    log::debug!(
                        "[host] lightmap layers ent={ent:#x} {}x{} -> id={}",
                        atlas.width,
                        atlas.height,
                        id
                    );
                    world.set(ent, lm_id_comp, LightmapId::new(id));
                } else if uploaded_atlas {
                    world.set(ent, lm_id_comp, LightmapId::new(0));
                }
                world.remove(ent, pending_lm_id);
            } else if uploaded_atlas && !world.has(ent, lm_id_comp) {
                world.set(ent, lm_id_comp, LightmapId::new(0));
            }
        }

        last_atlas_slot
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn flush_pending_albedos(&mut self) -> u32 { 0 }

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
        system_profiler::begin_frame();

        {
            let _g = system_profiler::scope("host.cmd");
            self.rebind_cmd_refs();
            stdin_cmd::drain(&self.cmd);
            self.drain_ui_commands();
            self.drain_channel_cmds();
            self.time_secs += delta_time.max(0.0);
            #[cfg(any(feature = "client", feature = "full"))]
            self.drain_pending_demo_cmds();
        }

        // Console input before gamelib so open console eats keys for freecam/etc.
        // ImGui draw runs later in [`Self::frame_ui`] (inside NewFrame).
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("host.console");
            let Host {
                console,
                cvar,
                cmd,
                client_engine,
                ..
            } = self;
            if let Some(client) = client_engine.as_mut() {
                console.tick_input(cvar, cmd, &mut client.input);
                let cons_open = console::Console::visible(cvar);
                binds::global().tick(&client.input, cmd, cons_open);
            }
        }

        #[cfg(any(feature = "client", feature = "full"))]
        {
            let _g = system_profiler::scope("client.dll");
            if let Some(ref dll) = self.client_dll {
                unsafe {
                    dll.frame(delta_time);
                }
            }
        }

        // UserCmd from this client frame → available to server.dll same tick.
        self.drain_channel_usercmds();

        {
            let _g = system_profiler::scope("host.net");
            self.tick_net(delta_time);
        }

        if self.server_engine.enabled {
            {
                let _g = system_profiler::scope("server.dll");
                if let Some(ref dll) = self.server_dll {
                    unsafe {
                        dll.frame(delta_time);
                    }
                }
            }
            {
                let _g = system_profiler::scope("server.engine");
                let _ = self.server_engine.frame(delta_time);
            }
        }

        {
            let _g = system_profiler::scope("host.replicate");
            #[cfg(any(feature = "client", feature = "full"))]
            if self.demo_player.is_some() {
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
            || self.demo_player.is_some()
            || (self.net.is_client() && self.client_session_phase != ClientSessionPhase::Idle)
        {
            let _g = system_profiler::scope("host.lightstyles");
            self.tick_lightstyles();
        }
        {
            let _g = system_profiler::scope("host.inspector");
            self.inspector.sync_world(&self.server_engine.world, delta_time);
            self.inspector.sync_assets(self.resources.list_assets());
            self.inspector
                .sync_systems(system_profiler::snapshot_for_inspector());
            self.sync_inspector_players();
            #[cfg(any(feature = "client", feature = "full"))]
            if let Some(client) = self.client_engine.as_ref() {
                let events = client
                    .audio_list_events()
                    .into_iter()
                    .map(|e| inspector::FmodEventSnap {
                        path: e.path,
                        bank: e.bank,
                        is_3d: e.is_3d,
                        oneshot: e.oneshot,
                        params: e.params,
                    })
                    .collect();
                self.inspector.sync_fmod_events(events);
            }
            self.inspector.poll(&self.server_engine.world);
        }

        system_profiler::end_frame();
    }

    /// ImGui overlays — call only between renderer `imgui_begin_frame` / end.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn frame_ui(&mut self) {
        let show_fps = self.cvar.get_usize_or("cl_showfps", 0) != 0
            || self.cvar.get_usize_or("r_showfps", 0) != 0;
        let show_pos = self.cvar.get_usize_or("cl_showpos", 0) as i32;
        let net_show = self.cvar.get_usize_or("net_show", 0) as i32;
        let view = self.local_view;
        let local_client_ent = self.local_client_entity();
        let net_hud = if net_show > 0 {
            Some(self.net_hud_info(net_show))
        } else {
            None
        };

        let Host {
            console,
            cvar,
            client_engine,
            server_engine,
            client_dll,
            ..
        } = self;
        let Some(client) = client_engine.as_ref() else {
            return;
        };
        console.tick_draw(cvar);
        let (view_yaw, view_pitch) = view.map(|v| (v.yaw, v.pitch)).unwrap_or((0.0, 0.0));
        deerpunk_engine_client::infobox::tick(
            show_fps,
            show_pos,
            &client.world,
            local_client_ent,
            view_yaw,
            view_pitch,
            net_hud.as_ref(),
        );

        // Client gamelib ImGui overlays (host-owned `imgui::Context`).
        if let Some(ref dll) = client_dll {
            let imgui_ctx = client.renderer().imgui_context_ptr();
            unsafe {
                dll.imgui_frame(imgui_ctx);
            }
        }
    }

    /// Mirrored client-world entity for the local player (if any).
    #[cfg(any(feature = "client", feature = "full"))]
    fn local_client_entity(&self) -> Option<u64> {
        if let Some(e) = self.client_rep.view_entity() {
            return Some(e);
        }
        let slot = self.players.local_slot()?;
        let sid = self.players.get(slot)?.entity?;
        let net_id = self.server_rep.net_id_of(sid)?;
        self.client_rep.client_of_net(net_id)
    }

    /// `NetworkEntity::net_id` for a player slot's pawn (after replication tick).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub fn player_net_id(&self, slot: u32) -> Option<u16> {
        let sid = self.players.get(slot)?.entity?;
        self.server_rep.net_id_of(sid)
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn client_set_view_net_id(&mut self, net_id: u16) {
        self.client_rep.set_view_net_id(net_id);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn client_set_lightstyle(&mut self, style: u8, pattern: &str) {
        self.client_lightstyles.set_pattern(style as usize, pattern);
    }

    /// Send a registered UserMessage (`slot = None` → broadcast remotes).
    pub fn um_send(
        &mut self,
        name: &str,
        data: &[u8],
        slot: Option<u32>,
    ) -> Result<(), String> {
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
        let Some(slot) = self.players.local_slot() else {
            return false;
        };
        self.slots_play.accepts_usercmd(slot)
    }

    /// S2C to one slot: UDP or in-process [`Channel`].
    fn deliver_s2c_to_slot(&mut self, slot: u32, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        let is_local = self.players.get(slot).map(|p| p.local).unwrap_or(false);
        if is_local {
            for p in packets {
                self.channel.push_server(p.clone());
            }
        } else {
            self.net.send_s2c_to_slot(slot, packets);
        }
    }

    /// Play-phase replication to all clients in **play** (UDP + local channel).
    fn deliver_s2c_broadcast(&mut self, packets: &[Vec<u8>]) {
        if packets.is_empty() {
            return;
        }
        self.net.broadcast_s2c(packets);
        if self.local_client_in_play() {
            for p in packets {
                self.channel.push_server(p.clone());
            }
        }
    }

    /// Join **init** bundle for one slot (then **play** via `InitDone`).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    fn send_join_init(&mut self, slot: u32) {
        let defs = self.server_rep.prefab_definition_packets(
            self.fs.gamedir(),
            &self.server_engine.world,
        );
        let snap = self.server_rep.snapshot(&self.server_engine.world);
        let def_count = defs.len();
        let snap_count = snap.len();
        self.deliver_s2c_to_slot(slot, &defs);
        if let Some(ref dll) = self.server_dll {
            unsafe {
                dll.join_init(slot, 0);
            }
        }
        self.deliver_s2c_to_slot(slot, &snap);
        if let Some(ref dll) = self.server_dll {
            unsafe {
                dll.join_init(slot, 1);
            }
        }
        self.deliver_s2c_to_slot(slot, &[vec![SVC::InitDone as u8]]);
        self.slots_play.mark_ready(slot);
        log::info!(
            "[net] init slot={slot}: {def_count} prefab defs, {snap_count} snap pkts → InitDone"
        );
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn net_hud_info(&self, level: i32) -> deerpunk_engine_client::infobox::NetHudInfo {
        let st = self.net.hud_stats();
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
            lines.push(format!(
                "loss: {:.1}%  remotes: {}  players: {}",
                st.packet_loss * 100.0,
                st.remote_clients,
                self.players.list().len()
            ));
            for p in self.players.list() {
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
            for (slot, addr) in self.net.remote_slot_debug() {
                lines.push(format!("  net slot {slot} ← {addr}"));
            }
        }
        deerpunk_engine_client::infobox::NetHudInfo { lines }
    }

    fn sync_inspector_players(&mut self) {
        let remote_addrs: std::collections::HashMap<u32, String> = self
            .net
            .remote_slot_debug()
            .into_iter()
            .collect();
        let players: Vec<serde_json::Value> = self
            .players
            .list()
            .iter()
            .map(|p| {
                #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
                let net_id = p.entity.and_then(|e| self.server_rep.net_id_of(e));
                #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
                let net_id: Option<u16> = None;
                serde_json::json!({
                    "index": p.slot,
                    "name": p.name,
                    "local": p.local,
                    "entity": p.entity,
                    "netId": net_id,
                    "addr": remote_addrs.get(&p.slot).cloned().unwrap_or_else(|| {
                        if p.local { "local".into() } else { "?".into() }
                    }),
                })
            })
            .collect();
        self.inspector.sync_players(serde_json::json!({
            "time": self.time_secs,
            "count": players.len(),
            "players": players,
            "role": self.net.hud_stats().role,
            "hosting": self.net.is_hosting(),
            "client": self.net.is_client(),
        }));
    }

    fn drain_channel(&mut self) {
        self.drain_channel_cmds();
        self.drain_channel_usercmds();
    }

    fn drain_channel_cmds(&mut self) {
        for cmd in self.channel.drain_client() {
            match cmd {
                ClientCmd::Map(name) => {
                    log::info!("[channel] map {name}");
                    self.set_server_enabled(true);
                    self.cmd.add_command(format!("map {name}"));
                    self.cmd.frame();
                }
                ClientCmd::Disconnect => {
                    log::info!("[channel] disconnect");
                    self.cmd.add_command("disconnect".into());
                    self.cmd.frame();
                    self.set_server_enabled(false);
                    move_state::clear();
                }
            }
        }
    }

    /// GS-style: one match for every CLC the server accepts.
    fn parse_client_message(&mut self, slot: u32, packet: &[u8]) {
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
                if self.server_engine.enabled && !self.slots_play.accepts_usercmd(slot) {
                    return;
                }
                let payload = &packet[1..];
                let Some(ucmd) = UserCmd::decode(payload) else {
                    log::warn!(
                        "[sv] bad UserCmd len={} from slot={slot}",
                        payload.len()
                    );
                    return;
                };
                self.players.set_usercmd(slot, ucmd);
            }
        }
    }

    fn drain_channel_usercmds(&mut self) {
        let slot = self.players.local_slot().unwrap_or(0);
        for packet in self.channel.drain_client_bytes() {
            self.parse_client_message(slot, &packet);
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn drain_pending_demo_cmds(&mut self) {
        let cmds = std::mem::take(&mut *PENDING_DEMO.lock().unwrap());
        for cmd in cmds {
            match cmd {
                PendingDemoCmd::Record(name) => self.demo_start_record(&name),
                PendingDemoCmd::Play(name) => self.demo_start_play(&name),
                PendingDemoCmd::Stop => self.demo_stop(),
            }
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn demo_start_record(&mut self, name: &str) {
        let Some(name) = demo::sanitize_name(name) else {
            println!("[demo] bad name (use alphanumeric/_/-)");
            return;
        };
        if self.demo_player.is_some() {
            self.demo_stop();
        }
        if self.demo_writer.is_some() {
            self.demo_stop();
        }
        let path = demo::demo_path(self.fs.gamedir(), &name);
        match demo::DemoWriter::create(path, self.time_secs) {
            Ok(w) => {
                println!("[demo] recording {}", w.path.display());
                self.demo_writer = Some(w);
                if self.server_engine.enabled {
                    self.connect_to_server_inprocess();
                }
            }
            Err(e) => println!("[demo] record failed: {e}"),
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn demo_start_play(&mut self, name: &str) {
        let Some(name) = demo::sanitize_name(name) else {
            println!("[demo] bad name (use alphanumeric/_/-)");
            return;
        };
        if self.demo_writer.is_some() {
            self.demo_stop();
        }
        if self.demo_player.is_some() {
            self.demo_stop();
        }
        let path = demo::demo_path(self.fs.gamedir(), &name);
        match demo::DemoPlayer::load(&path) {
            Ok(player) => {
                if self.server_engine.enabled {
                    self.set_server_enabled(false);
                }
                if self.net.is_hosting() || self.net.is_client() {
                    self.net.disconnect();
                }
                if let Some(client) = self.client_engine.as_mut() {
                    self.client_rep.clear(&client.world);
                }
                self.client_lightstyles = LightstyleTable::hl_defaults();
                let n = player.records.len();
                println!("[demo] playing {} ({} records)", path.display(), n);
                self.demo_player = Some(player);
                let _ = self.cvar.set("cl_demoplaying", "1");
                deerpunk_ui::set_client_in_game(true);
            }
            Err(e) => println!("[demo] play failed: {e}"),
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn demo_stop(&mut self) {
        if let Some(w) = self.demo_writer.take() {
            match w.finish() {
                Ok((path, n)) => println!("[demo] stopped recording {} ({} pkts)", path.display(), n),
                Err(e) => println!("[demo] stop record error: {e}"),
            }
        }
        if self.demo_player.take().is_some() {
            println!("[demo] stopped playback");
            if let Some(client) = self.client_engine.as_mut() {
                self.client_rep.clear(&client.world);
            }
            let _ = self.cvar.set("cl_demoplaying", "0");
            if !self.server_engine.enabled && !self.net.is_client() {
                deerpunk_ui::set_client_in_game(false);
            }
        }
    }

    pub fn alloc_sound_id(&mut self) -> u32 {
        let id = self.next_sound_id;
        self.next_sound_id = self.next_sound_id.wrapping_add(1);
        if self.next_sound_id == 0 {
            self.next_sound_id = 1;
        }
        id
    }

    pub fn queue_sound_packet(&mut self, packet: Vec<u8>) {
        self.channel.push_server(packet);
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
            let Some(ent) = self.client_rep.client_of_net(net_id) else {
                continue;
            };
            let ptr = client.world.get_raw(ent, t_id) as *const deerpunk_engine_common::components::TransformComponent;
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
                let _ = self.client_rep.apply_packet(
                    &client.world,
                    &gamedir,
                    &mut self.resources,
                    packet,
                );
            }
            SVC::UserMessage => self.dispatch_user_message_packet(packet),
            SVC::InitDone => {
                self.client_session_phase = ClientSessionPhase::Play;
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
        if self.demo_player.is_none() {
            let t = self.time_secs;
            if let Some(w) = self.demo_writer.as_mut() {
                if let Err(e) = w.write_svc(t, packet) {
                    log::warn!("[demo] write: {e}");
                }
            }
        }
        self.parse_server_message(packet);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn demo_record_view_frame(&mut self) {
        let Some(view) = self.local_view else {
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
        let Some(w) = self.demo_writer.as_mut() else {
            return;
        };
        if let Err(e) = w.write_view(t, dv) {
            log::warn!("[demo] view write: {e}");
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn tick_demo_playback(&mut self, dt: f32) {
        let packets = {
            let Some(player) = self.demo_player.as_mut() else {
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
                self.last_atlas_slot = s;
            }
        }
        if let Some(v) = self.demo_player.as_ref().and_then(|p| p.last_view) {
            self.local_view = Some(LocalViewStamp {
                yaw: v.yaw,
                pitch: v.pitch,
                roll: v.roll,
                eye_height: v.eye_height,
                fov_y: v.fov_y,
            });
        }
        if let Some(client) = self.client_engine.as_ref() {
            apply_local_view_to_client(
                &self.players,
                &self.server_rep,
                &self.client_rep,
                &client.world,
                self.local_view,
            );
        }
        let finished = self
            .demo_player
            .as_ref()
            .map(|p| p.finished())
            .unwrap_or(false);
        if finished {
            println!("[demo] finished");
            self.demo_stop();
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn drain_client_channel_s2c(&mut self) {
        for packet in self.channel.drain_server() {
            self.apply_local_s2c(&packet);
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn sync_client_world(&mut self) {
        if self.client_engine.is_none() {
            return;
        }

        if self.server_engine.enabled {
            let packets = self.server_rep.tick(&self.server_engine.world);
            self.deliver_s2c_broadcast(&packets);
        }

        self.drain_client_channel_s2c();

        let client_world =
            &self.client_engine.as_ref().unwrap().world as *const deerpunk_engine_common::world::World;
        let s = unsafe { self.flush_pending_albedos_world(&*client_world) };
        if s != 0 {
            self.last_atlas_slot = s;
        }

        let client = self.client_engine.as_ref().unwrap();
        apply_local_view_to_client(
            &self.players,
            &self.server_rep,
            &self.client_rep,
            &client.world,
            self.local_view,
        );
    }

    fn tick_net(&mut self, dt: f32) {
        self.drain_pending_net_cmds();
        self.tick_lan();
        let slots = self.players.slots();
        let duration = std::time::Duration::from_secs_f32(dt.max(0.0));
        let lan_reply = if self.net.is_hosting() {
            Some(self.lan_advertise_state())
        } else {
            None
        };
        let (s2c, c2s, need_public) =
            self.net.update(duration, &slots, lan_reply.as_ref());
        if let Some(addr) = need_public {
            self.net.apply_need_public_addr(addr);
        }

        if self.net.is_client()
            && self.net.client_connected()
            && self.client_session_phase == ClientSessionPhase::Idle
        {
            self.client_session_phase = ClientSessionPhase::Init;
            log::info!("[net] transport up → waiting init");
        }

        let udp_connects = std::mem::take(&mut self.net.pending_player_connect);
        for &slot in &udp_connects {
            self.players
                .connect_remote(slot, format!("player{slot}"));
            if let Some(ref dll) = self.server_dll {
                unsafe {
                    dll.player_connected(slot);
                }
            }
        }
        #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
        if !udp_connects.is_empty() {
            self.finish_client_join(&udp_connects);
        }
        let disconnects = std::mem::take(&mut self.net.pending_player_disconnect);
        for slot in disconnects {
            self.slots_play.remove(slot);
            if let Some(ref dll) = self.server_dll {
                unsafe {
                    dll.player_disconnected(slot);
                }
            }
            self.players.disconnect_slot(slot);
        }

        for (slot, packet) in c2s {
            self.parse_client_message(slot, &packet);
        }

        // Remote client: apply SVC packets from host (+ demo record).
        #[cfg(any(feature = "client", feature = "full"))]
        if self.net.is_client() && !s2c.is_empty() && self.demo_player.is_none() {
            for packet in &s2c {
                self.apply_local_s2c(packet);
            }
            if let Some(client) = self.client_engine.as_ref() {
                let world =
                    &client.world as *const deerpunk_engine_common::world::World;
                let s = unsafe { self.flush_pending_albedos_world(&*world) };
                if s != 0 {
                    self.last_atlas_slot = s;
                }
            }
        }
    }

    /// Apply client view stamp to the local mirrored pawn (angles + eye + fov).
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn apply_local_view(
        &mut self,
        yaw: f32,
        pitch: f32,
        roll: f32,
        eye_height: f32,
        fov_y: f32,
    ) {
        self.local_view = Some(LocalViewStamp {
            yaw,
            pitch,
            roll,
            eye_height,
            fov_y,
        });
        let Some(client) = self.client_engine.as_ref() else {
            return;
        };
        let cid = self.client_rep.view_entity().or_else(|| {
            let slot = self.players.local_slot()?;
            let sid = self.players.get(slot)?.entity?;
            let net_id = self.server_rep.net_id_of(sid)?;
            self.client_rep.client_of_net(net_id)
        });
        let Some(cid) = cid else {
            return;
        };
        stamp_view_on_entity(&client.world, cid, self.local_view.unwrap());
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn sync_client_world(&mut self) {}

    #[cfg(not(any(feature = "client", feature = "full")))]
    pub fn apply_local_view(
        &mut self,
        _yaw: f32,
        _pitch: f32,
        _roll: f32,
        _eye_height: f32,
        _fov_y: f32,
    ) {
    }

    /// Evaluate client lightstyle table → GPU scales (patterns via gamelib UM hooks).
    #[cfg(any(feature = "client", feature = "full"))]
    fn tick_lightstyles(&mut self) {
        if self.client_engine.is_none() {
            return;
        }
        self.client_lightstyles.tick(self.time_secs);
        let scales = self.client_lightstyles.scales_array();
        if let Some(client) = self.client_engine.as_mut() {
            client.renderer_mut().set_lightstyles(&scales);
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
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_time).as_secs_f32();
            last_time = now;

            stdin_cmd::drain(&self.cmd);
            self.drain_ui_commands();
            self.drain_channel();

            if REQUEST_QUIT.load(Ordering::SeqCst) {
                break;
            }

            let cons_open = self.cvar.get_bool_or("cl_console_show", false);
            self.client_engine
                .as_mut()
                .unwrap()
                .set_text_input_active(cons_open);

            // Audio cvars + queued `s_play` commands.
            {
                let client = self.client_engine.as_mut().unwrap();
                client.set_audio_volume(self.cvar.get_f32_or("s_volume", 1.0));
                for event in std::mem::take(&mut *S_PLAY_QUEUE.lock().unwrap()) {
                    client.audio_play_2d(&event, 1.0, &[]);
                    log::info!("[s_play] '{event}'");
                }
            }

            // Channel s2c (sounds, …) → same parse path as net / demo.
            {
                let packets = self.channel.drain_server();
                for packet in packets {
                    self.apply_local_s2c(&packet);
                }
            }

            // Poll first so client.dll mlook sees this frame's mouse before extract.
            if !self.client_engine.as_mut().unwrap().poll_events() {
                break;
            }

            // Server simulate only while enabled.
            if self.server_engine.enabled {
                self.server_engine.world.simulate(dt);
            }

            // client.dll (mlook + UserCmd) → server.dll → replicate + stamp local view.
            // Must run before extract so the camera uses this frame's viewangles.
            self.frame(dt);

            {
                let client = self.client_engine.as_mut().unwrap();
                let r = client.renderer_mut();
                r.set_fullbright(self.cvar.get_bool_or("r_fullbright", false));
                r.set_lightmap_only(self.cvar.get_bool_or("r_lightmap", false));
                let e = self.cvar.get_f32_or("r_exposure", 1.0);
                r.set_exposure_override(if e < 0.0 { None } else { Some(e) });
                r.set_tonemap_mode(self.cvar.get_f32_or("r_tonemap", -1.0));
                r.set_bloom(
                    self.cvar.get_f32_or("r_bloom", 1.0) >= 0.5,
                    self.cvar.get_f32_or("r_bloom_threshold", 0.7),
                    self.cvar.get_f32_or("r_bloom_intensity", 1.0),
                );
                r.set_grain(
                    self.cvar.get_f32_or("r_grain", 1.0) >= 0.5,
                    self.cvar.get_f32_or("r_grain_strength", 0.025),
                    self.cvar.get_f32_or("r_grain_downscale", 3.0).clamp(1.0, 8.0),
                );
            }

            {
                let client = self.client_engine.as_mut().unwrap();
                client.advance_time(dt);
            }
            self.sync_server_sound_origins();
            let view_ent = self.client_rep.view_entity();
            let scene = self
                .client_engine
                .as_mut()
                .unwrap()
                .extract_scene_view(view_ent);

            // Physics collider wireframe → RenderSnapshot.debug_draw (before GPU).
            {
                use deerpunk_engine_client::client_engine::DebugDrawBuf;
                let mut draw = DebugDrawBuf::new();
                if let Some(client) = self.client_engine.as_ref() {
                    phys_debug::tick(
                        &self.cvar,
                        &client.world,
                        &self.server_engine.world,
                        self.server_engine.physics.as_ref(),
                        &mut draw,
                    );
                    entity_debug::tick(&self.cvar, &client.world, &mut draw);
                }
                if let Some(client) = self.client_engine.as_mut() {
                    *client.debug_draw() = draw;
                }
            }

            // Render: ImGui NewFrame wraps frame_ui (console/debug overlays).
            let host_ptr = self as *mut Self as *mut std::ffi::c_void;
            if !self.client_engine.as_mut().unwrap().tick_inner(
                Some(&scene),
                Some(host_frame_ui_trampoline),
                host_ptr,
                false,
            ) {
                break;
            }
            if REQUEST_QUIT.load(Ordering::SeqCst) {
                break;
            }
            self.client_engine.as_mut().unwrap().input.reset_mouse_delta();
        }

        self.shutdown();
    }

    /// Persist configs and tear down DLLs. Safe to call once at exit.
    pub fn shutdown(&mut self) {
        config_io::write_all(&self.cvar, &self.fs);

        #[cfg(any(feature = "client", feature = "full"))]
        {
            if let Some(ref dll) = self.client_dll {
                unsafe {
                    dll.shutdown();
                }
            }
            self.client_dll = None;
            // Documents → context → Rml::Shutdown before window/renderer die.
            if let Some(client) = self.client_engine.as_mut() {
                client.shutdown_ui();
            }
        }
        if let Some(ref dll) = self.server_dll {
            unsafe {
                dll.shutdown();
            }
        }
        self.server_dll = None;
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

#[cfg(any(feature = "client", feature = "full"))]
fn apply_local_view_to_client(
    players: &players::PlayerConnections,
    server_rep: &deerpunk_engine_server::network::ServerPrefabReplication,
    client_rep: &deerpunk_engine_client::network::ClientPrefabReplication,
    client_world: &deerpunk_engine_common::world::World,
    stamp: Option<LocalViewStamp>,
) {
    // Prefer server UserMessage SetView target.
    let cid = client_rep.view_entity().or_else(|| {
        let slot = players.local_slot()?;
        let sid = players.get(slot)?.entity?;
        let net_id = server_rep.net_id_of(sid)?;
        client_rep.client_of_net(net_id)
    });
    let Some(cid) = cid else {
        return;
    };
    let stamp = stamp.unwrap_or_else(|| {
        let (yaw, pitch) = players
            .local_slot()
            .and_then(|s| players.get(s))
            .map(|p| (p.usercmd.yaw, p.usercmd.pitch))
            .unwrap_or((0.0, 0.0));
        LocalViewStamp {
            yaw,
            pitch,
            roll: 0.0,
            eye_height: deerpunk_engine_common::physics::CHAR_EYE_HEIGHT,
            fov_y: 60.0_f32.to_radians(),
        }
    });
    stamp_view_on_entity(client_world, cid, stamp);
}

#[cfg(any(feature = "client", feature = "full"))]
fn stamp_view_on_entity(
    client_world: &deerpunk_engine_common::world::World,
    entity: u64,
    stamp: LocalViewStamp,
) {
    use deerpunk_engine_common::components::{CameraComponent, TransformComponent};
    use glam::Quat;

    let tid = client_world.component_init::<TransformComponent>();
    let t_ptr = client_world.get_raw(entity, tid) as *mut TransformComponent;
    if !t_ptr.is_null() {
        unsafe {
            let t = &mut *t_ptr;
            let pitch = stamp.pitch.clamp(-1.5, 1.5);
            t.rotation = Quat::from_rotation_z(-stamp.yaw)
                * Quat::from_rotation_x(pitch)
                * Quat::from_rotation_y(stamp.roll);
        }
    }

    let cam_id = client_world.component_init::<CameraComponent>();
    let c_ptr = client_world.get_raw(entity, cam_id) as *mut CameraComponent;
    if !c_ptr.is_null() {
        unsafe {
            let c = &mut *c_ptr;
            c.eye_height = stamp.eye_height;
            c.fov_y = stamp.fov_y.max(1.0_f32.to_radians());
        }
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
