#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
use std::sync::atomic::{AtomicBool, Ordering};
pub type Entity = u64;

static REQUEST_QUIT: AtomicBool = AtomicBool::new(false);

pub mod bsp_debug_ui;
pub mod cmd;
pub mod console;
pub mod cvar;
pub mod fs;
pub mod inspector;
pub mod schedule;
pub mod scene_cmd;
pub mod stdin_cmd;
pub use schedule::Schedule;

pub mod api;
pub use api::{ComponentId, EntityId};

pub mod server_engine;
pub use server_engine::ServerEngine;

pub mod engine_api;
pub use deerpunk_engine_common::world;
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

inventory::submit! {
    cvar::CvarDef { name: "r_texturehandle", default: "", desc: "Shared memory texture handle for editor frame output" }
}
inventory::submit! {
    cvar::CvarDef { name: "gamedir", default: "examples/deathmatch/gamedir", desc: "Game directory path" }
}
inventory::submit! {
    cvar::CvarDef { name: "ipc_debug", default: "0", desc: "Enable IPC debug logging (1=log all messages)" }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_initial_scene",
        default: "",
        desc: "Scene name to load after gamelib init (e.g. menu). Empty = gamelib loads itself."
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_inspector_port",
        default: "9000",
        desc: "WebSocket port for deerpunk-inspector (0 = disabled).",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "sv_inspector_host",
        default: "127.0.0.1",
        desc: "Bind address for inspector WS (e.g. 127.0.0.1 or 0.0.0.0).",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "cl_console_show",
        default: "0",
        desc: "Show in-game ImGui console (1=visible). Toggle with ~.",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_fullbright",
        default: "0",
        desc: "Ignore lightmaps (1=albedo only, HL-style fullbright).",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_lightmap",
        default: "0",
        desc: "Debug: draw lightmap only (1=on).",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_bsp_debug",
        default: "0",
        desc: "Client BSP face pick: 0=off, 1=face dump, 2=+LM atlas layers + face rect.",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_exposure",
        default: "1",
        desc: "Exposure for tonemap. -1 = auto-exposure, otherwise fixed multiplier. 1 = GoldSrc-style.",
    }
}
inventory::submit! {
    cvar::CvarDef {
        name: "r_tonemap",
        default: "-1",
        desc: "Tonemap mode: -1=passthrough (GoldSrc-style raw), 0=ACES, 1=Reinhard, 2=Hable.",
    }
}

#[cfg(any(feature = "client", feature = "full"))]
extern "C" fn host_frame_trampoline(ctx: *mut std::ffi::c_void) {
    let host = unsafe { &mut *(ctx as *mut Host) };
    host.frame(0.016);
}

// ── Host ─────────────────────────────────────────────────────

pub struct Host {
    pub server_engine: ServerEngine,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<ClientEngine>,
    pub cvar: cvar::CvarSystem,
    pub cmd: cmd::CmdSystem,
    pub fs: fs::FileSystem,
    pub resources: deerpunk_engine_common::resource::ResourceManager,
    pub channel: Channel,
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
            resources,
            channel: Channel::new(),
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
        scene_cmd::register(&mut self.cmd);
        self.cmd.register_command("quit", |_| {
            REQUEST_QUIT.store(true, Ordering::SeqCst);
        });
        let autoexec = self.fs.resolve_path("configs/autoexec.cfg");
        self.cmd
            .exec_config_file(&autoexec.to_string_lossy());
        self.register_internal_components();

        // UI runtime before gamelib so `v1:ui:*` works from server_init.
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let gamedir_ui = self.fs.resolve_ui_dir();
            if let Some(client) = self.client_engine.as_mut() {
                client.init_ui(gamedir_ui);
            }
        }

        let port = self
            .cvar
            .get_string("sv_inspector_port")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(9000);
        let host = self
            .cvar
            .get_string("sv_inspector_host")
            .unwrap_or_else(|| "127.0.0.1".into());
        log::info!("[inspector] cfg host={host} port={port}");
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
        inspector::start(&host, port);
        stdin_cmd::start();

        log::info!("Engine initialized");
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
        self.server_engine.set_enabled(on);
        if !on {
            self.replicate_full = true;
        }
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
        self.rebind_cmd_refs();
        stdin_cmd::drain(&self.cmd);
        self.drain_ui_commands();
        self.drain_channel();
        self.drain_scene_cmd();
        self.time_secs += delta_time.max(0.0);

        // Console before gamelib so open console eats keys for freecam/etc.
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let Host {
                console,
                cvar,
                cmd,
                client_engine,
                server_engine,
                ..
            } = self;
            if let Some(client) = client_engine.as_mut() {
                console.tick(cvar, cmd, &mut client.input);
                bsp_debug_ui::tick(cvar, &client.world, &server_engine.world);
            }
        }

        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(ref dll) = self.client_dll {
            unsafe {
                dll.frame(delta_time);
            }
        }

        if self.server_engine.enabled {
            if let Some(ref dll) = self.server_dll {
                unsafe {
                    dll.frame(delta_time);
                }
            }
            let _ = self.server_engine.frame(delta_time);
            self.tick_lightstyles();
        }

        self.sync_client_world();
        inspector::sync_world(&self.server_engine.world, delta_time);
        inspector::sync_assets(self.resources.list_assets());
    }

    fn drain_channel(&mut self) {
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
                }
            }
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
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn sync_client_world(&mut self) {}

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
        let host_ptr = self as *mut Self as *mut std::ffi::c_void;
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

            // Server simulate only while enabled (also runs inside host.frame via trampoline).
            if self.server_engine.enabled {
                self.server_engine.world.simulate(dt);
            }
            self.sync_client_world();

            {
                let client = self.client_engine.as_mut().unwrap();
                let r = client.renderer_mut();
                r.set_fullbright(self.cvar.get_bool_or("r_fullbright", false));
                r.set_lightmap_only(self.cvar.get_bool_or("r_lightmap", false));
                let e = self.cvar.get_f32_or("r_exposure", 1.0);
                r.set_exposure_override(if e < 0.0 { None } else { Some(e) });
                r.set_tonemap_mode(self.cvar.get_f32_or("r_tonemap", -1.0));
            }

            let scene = self.client_engine.as_ref().unwrap().extract_scene();

            if !self.client_engine.as_mut().unwrap().tick(
                Some(&scene),
                Some(host_frame_trampoline),
                host_ptr,
            ) {
                break;
            }
            if REQUEST_QUIT.load(Ordering::SeqCst) {
                break;
            }
            self.client_engine.as_mut().unwrap().input.reset_mouse_delta();
        }
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
