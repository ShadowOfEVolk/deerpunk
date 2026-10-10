#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
pub type Entity = u64;

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

pub mod server_dll;
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
    server_dll: Option<ServerDll>,
    #[cfg(any(feature = "client", feature = "full"))]
    console: console::Console,
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
            server_dll: None,
            #[cfg(any(feature = "client", feature = "full"))]
            console: console::Console::new(),
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
        deerpunk_engine_common::resource::register_default_loaders(
            &mut self.resources,
            gamedir.as_ref(),
        );
        self.cvar.initialize();
        self.cmd.initialize();
        scene_cmd::register(&mut self.cmd);
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
        let w = &self.server_engine.world;
        w.component_init::<TransformComponent>();
        w.component_init::<Mesh3D>();
        w.component_init::<Material>();
        w.component_init::<CameraComponent>();
        w.component_init::<SimpleEntity>();
        w.component_init::<UiDocumentComponent>();
        // Client extract queries reference these even if the game never spawns them.
        w.component_init::<Terrain>();
        w.component_init::<TerrainSplatMaterial>();
        w.component_init::<Worldspawn>();
        w.component_init::<PendingBspArrays>();
        w.component_init::<MeshParts>();
        w.component_init::<MapKv>();
        w.component_init::<PointLight>();
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
                log::error!("DLL load failed: {}. Running headless.", e);
            }
        }

        self.load_initial_scene_from_cvar();
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
        Ok(entity)
    }

    /// Upload [`PendingBspArrays`] → GPU texture arrays + [`MeshParts`].
    #[cfg(any(feature = "client", feature = "full"))]
    fn flush_pending_albedos(&mut self) {
        use deerpunk_engine_common::components::{
            MeshPart, MeshParts, PendingBspArrays,
        };

        let Some(client) = self.client_engine.as_mut() else {
            return;
        };
        let world = &self.server_engine.world;
        let pending_id = world.component_init::<PendingBspArrays>();
        let parts_id = world.component_init::<MeshParts>();

        let ents: Vec<u64> = world
            .inspect_entities()
            .into_iter()
            .filter(|e| e.components.iter().any(|c| c.contains("PendingBspArrays")))
            .map(|e| e.id)
            .collect();

        for ent in ents {
            let ptr = world.get_raw(ent, pending_id) as *mut PendingBspArrays;
            if ptr.is_null() {
                continue;
            }
            let pending = unsafe { std::ptr::read(ptr) };
            // Leave empty shell in flecs before remove.
            unsafe {
                std::ptr::write(ptr, PendingBspArrays::default());
            }

            let mut bucket_to_array: Vec<u32> = Vec::with_capacity(pending.buckets.len());
            for b in &pending.buckets {
                let layers: Vec<Vec<u8>> = b.layers.iter().map(|l| l.pixels.clone()).collect();
                let id = client
                    .renderer_mut()
                    .upload_texture_array(b.size, &layers);
                log::info!(
                    "[host] tex array ent={ent:#x} size={} layers={} -> id={}",
                    b.size,
                    layers.len(),
                    id
                );
                bucket_to_array.push(id);
            }

            let parts: Vec<MeshPart> = pending
                .parts
                .iter()
                .filter_map(|p| {
                    let array_id = *bucket_to_array.get(p.bucket as usize)?;
                    Some(MeshPart {
                        first_index: p.first_index,
                        index_count: p.index_count,
                        array_id,
                        layer: p.layer,
                    })
                })
                .collect();

            world.set(ent, parts_id, MeshParts::new(parts));
            world.remove(ent, pending_id);
            // pending dropped here (owns bucket pixel data)
            drop(pending);
        }
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    fn flush_pending_albedos(&mut self) {}

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
        self.drain_scene_cmd();

        // Console before gamelib so open console eats keys for freecam/etc.
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let Host {
                console,
                cvar,
                cmd,
                client_engine,
                ..
            } = self;
            if let Some(client) = client_engine.as_mut() {
                console.tick(cvar, cmd, &mut client.input);
            }
        }

        if let Some(ref dll) = self.server_dll {
            unsafe {
                dll.frame(delta_time);
            }
        }
        let _ = self.server_engine.frame(delta_time);
        inspector::sync_world(&self.server_engine.world, delta_time);
        inspector::sync_assets(self.resources.list_assets());
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn run(&mut self) {
        if self.client_engine.is_none() {
            return;
        }

        self.server_engine.world.simulate(0.0);

        let world_ptr = self.server_engine.world.as_ptr();
        deerpunk_engine_common::world::install_panic_hook(world_ptr);

        self.server_engine.world.debug_dump(&[
            "TransformComponent",
            "Mesh3D",
            "Material",
            "CameraComponent",
            "SimpleEntity",
            "UiDocumentComponent",
        ]);

        self.client_engine.as_mut().unwrap().init_extractor(&mut self.server_engine.world);

        let mut last_time = std::time::Instant::now();
        let host_ptr = self as *mut Self as *mut std::ffi::c_void;
        loop {
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_time).as_secs_f32();
            last_time = now;

            stdin_cmd::drain(&self.cmd);
            self.drain_scene_cmd();

            let cons_open = self.cvar.get_bool_or("cl_console_show", false);
            self.client_engine
                .as_mut()
                .unwrap()
                .set_text_input_active(cons_open);

            self.server_engine.world.simulate(dt);

            let scene = self.client_engine.as_ref().unwrap().extract(&self.server_engine.world);

            if !self.client_engine.as_mut().unwrap().tick(
                Some(&scene),
                Some(host_frame_trampoline),
                host_ptr,
            ) {
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
