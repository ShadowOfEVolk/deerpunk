#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;
#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::render_extractor;
use deerpunk_engine_common::components::{CameraComponent, MeshRenderer, TransformComponent};
pub type Entity = u64;

pub mod cmd;
pub mod cvar;
pub mod fs;
pub mod schedule;
pub use schedule::Schedule;

pub mod api;
pub use api::{ComponentId, EntityId};

pub mod vtable;
pub use vtable::{EngineApiHandle, EngineApiVtable};

pub mod server_engine;
pub use server_engine::ServerEngine;

pub mod engine_api;
pub use deerpunk_engine_common::world;
pub use engine_api::imgui::*;
pub use engine_api::input::*;
pub use engine_api::physics::*;
pub use engine_api::world::*;

pub mod server_dll;
pub use server_dll::{ResolverFn, ServerDll, ServerInitData};

pub mod common {
    pub use deerpunk_engine_common::*;
    pub mod input {
        pub use deerpunk_engine_common::input::*;
    }
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

// ── Host ─────────────────────────────────────────────────────

pub struct Host {
    pub server_engine: ServerEngine,
    #[cfg(any(feature = "client", feature = "full"))]
    pub client_engine: Option<ClientEngine>,
    pub cvar: cvar::CvarSystem,
    pub cmd: cmd::CmdSystem,
    pub fs: fs::FileSystem,
    server_dll: Option<ServerDll>,
}

impl Host {
    pub fn new() -> Self {
        let mut cmd_system = cmd::CmdSystem::new();
        let cvar_system = cvar::CvarSystem::new();
        let fs_system = fs::FileSystem::new();
        cmd_system.set_refs(&cvar_system, &fs_system);
        Self {
            server_engine: ServerEngine::new(),
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine: Some(ClientEngine::new()),
            cvar: cvar_system,
            cmd: cmd_system,
            fs: fs_system,
            server_dll: None,
        }
    }

    pub fn initialize(&mut self, gamedir: impl AsRef<std::path::Path>) {
        self.fs.initialize();
        self.fs.set_gamedir(gamedir);
        self.cvar.initialize();
        self.cmd.initialize();
        self.cmd.exec_config_file("configs/autoexec.cfg");
        self.register_internal_components();

        log::info!("Engine initialized");
    }

    fn register_internal_components(&mut self) {
        use deerpunk_engine_common::components::*;
        let w = &self.server_engine.world;
        w.component_init::<TransformComponent>();
        w.component_init::<MeshRenderer>();
        w.component_init::<CameraComponent>();
        w.component_init::<SimpleEntity>();
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
    }

    pub fn frame(&self, delta_time: f32) {
        if let Some(ref dll) = self.server_dll {
            unsafe {
                dll.frame(delta_time);
            }
        }
        let _ = self.server_engine.frame(delta_time);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn run(&mut self) {
        let mut client = match self.client_engine.take() {
            Some(c) => c,
            None => return,
        };

        self.server_engine.world.simulate(0.0);

        let world_ptr = self.server_engine.world.as_ptr();
        deerpunk_engine_common::world::install_panic_hook(world_ptr);

        self.server_engine.world.debug_dump(&[
            "TransformComponent",
            "MeshRenderer",
            "CameraComponent",
            "SimpleEntity",
        ]);

        client.init_extractor(&mut self.server_engine.world);

        let mut last_time = std::time::Instant::now();
        loop {
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_time).as_secs_f32();
            last_time = now;

            self.server_engine.world.simulate(dt);

            let scene = client.extract(&self.server_engine.world);

            if !client.tick(Some(&scene)) {
                break;
            }
            self.frame(0.016);
        }

        self.client_engine = Some(client);
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}
