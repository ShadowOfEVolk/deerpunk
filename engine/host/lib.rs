#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
use std::sync::atomic::{AtomicBool, Ordering};
pub type Entity = u64;

static REQUEST_QUIT: AtomicBool = AtomicBool::new(false);
/// `s_play <event>` console commands queued for the client frame.
static S_PLAY_QUEUE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub mod binds;
pub mod bsp_debug_ui;
pub mod config_io;
pub mod fps_ui;
pub mod cmd;
pub mod console;
pub mod cvar;
pub mod fs;
pub mod gameinfo;
pub mod inspector;
pub mod move_state;
pub mod schedule;
pub mod scene_cmd;
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
use deerpunk_engine_common::network::sound_event::{
    SoundEventRpc, KIND_PLAY_2D, KIND_PLAY_3D, KIND_START_ATTACHED, KIND_STOP_ATTACHED,
};
use deerpunk_engine_common::network::UserCmd;
pub use engine_api::cmd::*;
pub use engine_api::imgui::*;
pub use engine_api::input::*;
pub use engine_api::physics::*;
pub use engine_api::resources::*;
pub use engine_api::scene::*;
pub use engine_api::ui::*;
pub use engine_api::world::*;

pub mod channel;
pub mod client_dll;
pub mod replicate;
pub mod server_dll;
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
        name: "sv_initial_scene",
        default: "",
        desc: "Scene name to load after gamelib init (e.g. menu). Empty = gamelib loads itself.",
        flags: 0,
    }
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
        name: "r_bsp_debug",
        default: "0",
        desc: "Client BSP face pick: 0=off, 1=face dump, 2=+LM atlas layers + face rect.",
        flags: cvar::RENDERINFO,
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_showfps",
        default: "0",
        desc: "Show FPS counter in the top-left corner (1=on).",
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
    #[cfg(any(feature = "client", feature = "full"))]
    replication: replicate::ReplicationState,
    /// Accumulated host time for lightstyle animation (`a`..`z`).
    time_secs: f32,
    /// Full client-world resync needed (after map / disconnect).
    replicate_full: bool,
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
            #[cfg(any(feature = "client", feature = "full"))]
            replication: replicate::ReplicationState::default(),
            time_secs: 0.0,
            replicate_full: false,
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
        scene_cmd::register(&mut self.cmd);
        self.cmd.register_command("quit", |_| {
            REQUEST_QUIT.store(true, Ordering::SeqCst);
        });
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
        inspector::register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
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
        inspector::register_collection(deerpunk_inspector_protocol::CollectionDescriptor {
            id: "systems".into(),
            label: "Systems".into(),
            group: deerpunk_inspector_protocol::CollectionGroup::Runtime,
            modes: vec![deerpunk_inspector_protocol::CollectionMode::List],
            id_field: "name".into(),
            nested_in: None,
        });
        stdin_cmd::start();

        log::info!("Engine initialized");
    }

    /// Start inspector WS using current cvars (after configs / stuffcmds).
    pub fn start_inspector(&self) {
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
        inspector::start(&host, port);
    }

    fn register_internal_components(&mut self) {
        use deerpunk_engine_common::components::*;
        let register = |w: &deerpunk_engine_common::world::World| {
            w.component_init::<TransformComponent>();
            w.component_init::<Mesh3D>();
            w.component_init::<Material>();
            w.component_init::<CameraComponent>();
            w.component_init::<SimpleEntity>();
            w.component_init::<UiDocumentComponent>();
            w.component_init::<Terrain>();
            w.component_init::<TerrainSplatMaterial>();
            w.component_init::<Worldspawn>();
            w.component_init::<PendingBspAtlas>();
            w.component_init::<PendingLightmap>();
            w.component_init::<LightmapId>();
            w.component_init::<LightstyleTable>();
            w.component_init::<BspWorldDebug>();
            w.component_init::<MapKv>();
            w.component_init::<PointLight>();
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

        self.load_initial_scene_from_cvar();

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

    pub fn set_server_enabled(&mut self, on: bool) {
        let was = self.server_engine.enabled;
        self.server_engine.set_enabled(on);
        if on {
            // Listen server: host wires local client ↔ server after map is up.
            if was {
                self.disconnect_local_client();
            }
            self.connect_local_client();
            self.replicate_full = true;
        } else if was {
            self.disconnect_local_client();
            self.replicate_full = true;
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
            let slot = match self.players.connect_local(name) {
                Some(c) => c.slot,
                None => return,
            };
            if let Some(ref dll) = self.server_dll {
                unsafe {
                    dll.player_connected(slot);
                }
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
    }

    /// Spawn `{gamedir}/prefabs/{name}.prefab` into the server world.
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub fn spawn_prefab(&mut self, name: &str) -> Result<u64, String> {
        let gamedir = self.fs.gamedir().to_path_buf();
        deerpunk_engine_server::spawn_prefab(
            &self.server_engine.world,
            &gamedir,
            &mut self.resources,
            name,
        )
        .map_err(|e| e.message)
    }

    #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
    pub fn spawn_prefab(&mut self, _name: &str) -> Result<u64, String> {
        Err("spawn_prefab unavailable".into())
    }

    /// Load a scene file `{gamedir}/scenes/{name}.scene` (server-owned).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub fn load_scene(&mut self, name: &str) -> Result<Vec<u64>, String> {
        let gamedir = self.fs.gamedir().to_path_buf();
        log::info!("[host] load_scene '{name}' gamedir={}", gamedir.display());
        let ids = deerpunk_engine_server::load_scene(
            &self.server_engine.world,
            &gamedir,
            &mut self.resources,
            name,
        )
        .map_err(|e| e.message)?;
        log::info!("[host] load_scene '{name}' ok — {} entities", ids.len());
        self.flush_pending_albedos();
        // Help verify Worldspawn/BSP in the console too.
        self.server_engine.world.debug_dump(&["Worldspawn", "Mesh3D", "TransformComponent"]);
        Ok(ids)
    }

    /// Load a BSP via ResourceManager. Returns asset id (`BspMap.id`).
    ///
    /// `path` may be a bare map name (`oldlifehome`), `maps/foo.bsp`, or absolute path.
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub fn load_bsp(&mut self, path: &str) -> Result<u64, String> {
        use deerpunk_engine_common::bsp30::BspWorldMesh;
        use deerpunk_engine_common::resource::{path_to_file_uri, AssetId, LoadOpts};

        let gamedir = self.fs.gamedir().to_path_buf();
        let bsp_path = resolve_bsp_path(&gamedir, path);
        let uri = path_to_file_uri(&bsp_path);
        log::info!("[host] load_bsp path={} uri={uri}", bsp_path.display());
        let handle = self
            .resources
            .load_with::<BspWorldMesh>(&uri, LoadOpts::default().loader("bsp"));
        if !handle.is_loaded() {
            return Err(format!(
                "bsp load failed: {} (asset={:?})",
                bsp_path.display(),
                handle.id()
            ));
        }
        let AssetId(id) = handle.id();
        Ok(id)
    }

    /// Place a previously loaded BSP asset into the world (geometry + map entities).
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    pub fn place_bsp(&mut self, asset_id: u64, map_name: &str) -> Result<u64, String> {
        use deerpunk_engine_common::bsp30::BspWorldMesh;
        use deerpunk_engine_common::resource::AssetId;

        let handle = self
            .resources
            .handle::<BspWorldMesh>(AssetId(asset_id))
            .ok_or_else(|| format!("unknown bsp asset id={asset_id}"))?;
        let loaded = handle
            .try_get()
            .ok_or_else(|| format!("bsp asset id={asset_id} not loaded"))?;

        let gamedir = self.fs.gamedir().to_path_buf();
        let entity = self.server_engine.world.spawn();
        deerpunk_engine_server::place_bsp_map(
            &self.server_engine.world,
            &gamedir,
            &mut self.resources,
            entity,
            loaded,
            map_name,
        )?;
        self.flush_pending_albedos();
        self.replicate_full = true;
        Ok(entity)
    }

    /// Upload [`PendingBspAtlas`] → Material.albedo + [`PendingLightmap`] → [`LightmapId`].
    #[cfg(any(feature = "client", feature = "full"))]
    fn flush_pending_albedos(&mut self) {
        use deerpunk_engine_common::components::{
            LightmapId, Material, PendingBspAtlas, PendingLightmap,
        };

        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        let world = &self.server_engine.world;
        let pending_id = world.component_init::<PendingBspAtlas>();
        let pending_lm_id = world.component_init::<PendingLightmap>();
        let lm_id_comp = world.component_init::<LightmapId>();
        let mat_id = world.component_init::<Material>();

        let ents: Vec<u64> = world
            .inspect_entities()
            .into_iter()
            .filter(|e| {
                e.components.iter().any(|c| {
                    c.contains("PendingBspAtlas") || c.contains("PendingLightmap")
                })
            })
            .map(|e| e.id)
            .collect();

        for ent in ents {
            let mut uploaded_atlas = false;
            let ptr = world.get_raw(ent, pending_id) as *mut PendingBspAtlas;
            if !ptr.is_null() {
                let pending = unsafe { std::ptr::read(ptr) };
                unsafe {
                    std::ptr::write(ptr, PendingBspAtlas::default());
                }
                if let Some(img) = pending.image {
                    let tex = client
                        .renderer_mut()
                        .upload_texture(&img.pixels, img.width, img.height);
                    log::info!(
                        "[host] albedo atlas ent={ent:#x} {}x{} -> slot={}",
                        img.width,
                        img.height,
                        tex.0
                    );
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
                    log::info!(
                        "[host] lightmap layers ent={ent:#x} {}x{} -> id={}",
                        atlas.width,
                        atlas.height,
                        id
                    );
                    world.set(ent, lm_id_comp, LightmapId::new(id));
                } else if uploaded_atlas {
                    // White LM fallback so extractor still uses atlas+LM pipeline.
                    world.set(ent, lm_id_comp, LightmapId::new(0));
                }
                world.remove(ent, pending_lm_id);
            } else if uploaded_atlas && !world.has(ent, lm_id_comp) {
                world.set(ent, lm_id_comp, LightmapId::new(0));
            }
        }
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn flush_pending_albedos(&mut self) {}

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

    fn drain_scene_cmd(&mut self) {
        #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
        {
            if let Some(name) = scene_cmd::take_pending() {
                match self.load_scene(&name) {
                    Ok(ids) => println!("[cmd] scene {name}: spawned {}", ids.len()),
                    Err(e) => println!("[cmd] scene {name}: FAILED — {e}"),
                }
            }
        }
    }

    fn load_initial_scene_from_cvar(&mut self) {
        #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
        {
            let Some(name) = self.cvar.get_string("sv_initial_scene") else {
                return;
            };
            let name = name.trim();
            if name.is_empty() {
                return;
            }
            match self.load_scene(name) {
                Ok(ids) => log::info!("[host] sv_initial_scene={name} spawned {}", ids.len()),
                Err(e) => log::error!("[host] sv_initial_scene={name} failed: {e}"),
            }
        }
    }

    pub fn frame(&mut self, delta_time: f32) {
        system_profiler::begin_frame();

        {
            let _g = system_profiler::scope("host.cmd");
            self.rebind_cmd_refs();
            stdin_cmd::drain(&self.cmd);
            self.drain_ui_commands();
            self.drain_channel_cmds();
            self.drain_scene_cmd();
            self.time_secs += delta_time.max(0.0);
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
            {
                let _g = system_profiler::scope("host.lightstyles");
                self.tick_lightstyles();
            }
        }

        {
            let _g = system_profiler::scope("host.replicate");
            self.sync_client_world();
        }
        {
            let _g = system_profiler::scope("host.inspector");
            inspector::sync_world(&self.server_engine.world, delta_time);
            inspector::sync_assets(self.resources.list_assets());
            inspector::sync_systems(system_profiler::snapshot_for_inspector());
        }

        system_profiler::end_frame();
    }

    /// ImGui overlays — call only between renderer `imgui_begin_frame` / end.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn frame_ui(&mut self) {
        let Host {
            console,
            cvar,
            client_engine,
            server_engine,
            ..
        } = self;
        let Some(client) = client_engine.as_ref() else {
            return;
        };
        console.tick_draw(cvar);
        bsp_debug_ui::tick(cvar, &client.world, &server_engine.world);
        fps_ui::tick(cvar);
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

    fn drain_channel_usercmds(&mut self) {
        for packet in self.channel.drain_client_bytes() {
            let Some((&clc, payload)) = packet.split_first() else {
                continue;
            };
            if clc != CLC::UserCmd as u8 {
                log::warn!("[channel] unknown CLC={clc}");
                continue;
            }
            let Some(cmd) = UserCmd::decode(payload) else {
                log::warn!("[channel] bad UserCmd payload len={}", payload.len());
                continue;
            };
            let slot = self.players.local_slot().unwrap_or(0);
            self.players.set_usercmd(slot, cmd);
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn sync_client_world(&mut self) {
        let enabled = self.server_engine.enabled;
        let Host {
            server_engine,
            client_engine,
            replication,
            replicate_full,
            players,
            ..
        } = self;
        let Some(client) = client_engine.as_mut() else {
            return;
        };
        if !enabled {
            if *replicate_full {
                replication.clear(&client.world);
                *replicate_full = false;
            }
            return;
        }
        if *replicate_full {
            replication.sync_full(&server_engine.world, &client.world);
            *replicate_full = false;
            log::info!("[replicate] full sync → client world");
        } else {
            replication.sync_transforms(&server_engine.world, &client.world);
        }
        // mlook is client-owned: stamp local viewangles onto the mirrored pawn cam.
        apply_local_view_to_client(players, replication, &client.world);
    }

    /// Apply absolute viewangles to the local player's client-mirrored transform.
    #[cfg(any(feature = "client", feature = "full"))]
    pub fn apply_local_view(&mut self, yaw: f32, pitch: f32) {
        let Some(slot) = self.players.local_slot() else {
            return;
        };
        let Some(sid) = self.players.get(slot).and_then(|p| p.entity) else {
            return;
        };
        let Some(client) = self.client_engine.as_ref() else {
            return;
        };
        let Some(cid) = self.replication.client_of(sid) else {
            return;
        };
        stamp_view_on_entity(&client.world, cid, yaw, pitch);
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn sync_client_world(&mut self) {}

    #[cfg(not(any(feature = "client", feature = "full")))]
    pub fn apply_local_view(&mut self, _yaw: f32, _pitch: f32) {}

    /// Evaluate [`LightstyleTable`] patterns and upload scales to the GPU.
    #[cfg(any(feature = "client", feature = "full"))]
    fn tick_lightstyles(&mut self) {
        use deerpunk_engine_common::components::LightstyleTable;

        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        let world = &self.server_engine.world;
        let ls_id = world.component_init::<LightstyleTable>();
        let time = self.time_secs;

        let ents: Vec<u64> = world
            .inspect_entities()
            .into_iter()
            .filter(|e| e.components.iter().any(|c| c.contains("LightstyleTable")))
            .map(|e| e.id)
            .collect();

        for ent in ents {
            let ptr = world.get_raw(ent, ls_id) as *mut LightstyleTable;
            if ptr.is_null() {
                continue;
            }
            let table = unsafe { &mut *ptr };
            table.tick(time);
            client.renderer_mut().set_lightstyles(&table.scales_array());
            // One table drives global styles UBO (last wins if several).
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
            self.drain_scene_cmd();

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

            // Apply queued server→client sound RPCs (binary packets; the
            // in-process stand-in for a real network channel).
            {
                let client = self.client_engine.as_mut().unwrap();
                for packet in self.channel.drain_server() {
                    let Some((&cmd, payload)) = packet.split_first() else {
                        continue;
                    };
                    if cmd != SVC::SoundEvent as u8 {
                        log::warn!("[net] unexpected s2c packet cmd={cmd}");
                        continue;
                    }
                    match SoundEventRpc::decode(payload) {
                        Some(rpc) => {
                            let params: Vec<(&str, f32)> = rpc
                                .params
                                .iter()
                                .map(|(n, v)| (n.as_str(), *v))
                                .collect();
                            match rpc.kind {
                                KIND_PLAY_2D => {
                                    client.audio_play_2d(&rpc.event, rpc.volume, &params)
                                }
                                KIND_PLAY_3D => {
                                    log::info!(
                                        "[net] sound rpc play_3d '{}'{:?} vol={:.2} params={:?}",
                                        rpc.event,
                                        rpc.position,
                                        rpc.volume,
                                        params
                                    );
                                    client.audio_play_3d(
                                        &rpc.event,
                                        rpc.position,
                                        rpc.volume,
                                        &params,
                                    )
                                }
                                KIND_START_ATTACHED => client.audio_start_attached(
                                    rpc.key,
                                    &rpc.event,
                                    rpc.position,
                                    rpc.volume,
                                ),
                                KIND_STOP_ATTACHED => client.audio_stop_attached(rpc.key),
                                other => log::warn!("[net] sound rpc kind={other}?"),
                            }
                        }
                        None => log::warn!("[net] sound rpc decode failed"),
                    }
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
            let scene = self.client_engine.as_ref().unwrap().extract_scene();

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
    replication: &replicate::ReplicationState,
    client_world: &deerpunk_engine_common::world::World,
) {
    let Some(slot) = players.local_slot() else {
        return;
    };
    let Some(p) = players.get(slot) else {
        return;
    };
    let Some(sid) = p.entity else {
        return;
    };
    let Some(cid) = replication.client_of(sid) else {
        return;
    };
    stamp_view_on_entity(client_world, cid, p.usercmd.yaw, p.usercmd.pitch);
}

#[cfg(any(feature = "client", feature = "full"))]
fn stamp_view_on_entity(
    client_world: &deerpunk_engine_common::world::World,
    entity: u64,
    yaw: f32,
    pitch: f32,
) {
    use deerpunk_engine_common::components::TransformComponent;
    use glam::Quat;

    let tid = client_world.component_init::<TransformComponent>();
    let ptr = client_world.get_raw(entity, tid) as *mut TransformComponent;
    if ptr.is_null() {
        return;
    }
    unsafe {
        let t = &mut *ptr;
        t.rotation = Quat::from_rotation_z(-yaw) * Quat::from_rotation_x(pitch.clamp(-1.5, 1.5));
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
