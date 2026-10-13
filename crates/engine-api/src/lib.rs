pub mod app;
pub mod audio;
pub mod cmd;
pub mod cvar;
pub mod input;
pub mod net;
pub mod physics;
pub mod renderer;
pub mod host_access;
pub mod components;
pub mod dynamic_load;
pub mod resources;
pub mod schedule;
pub mod ui;
pub mod version;
pub mod methoddb;
pub mod world;
pub mod client_world;
pub mod spawn_prefab;
pub mod materials;

pub use dynamic_load::{drop_dynamic, pack as pack_dynamic_load, DynamicLoad};
pub use deerpunk_engine_ffi::{
    ClientInitData, ComponentId, EntityId, GAMELIB_API_VERSION, GamelibInitData, ResolverFn,
    ServerInitData,
};

/// Alias for [`ResolverFn`] (methoddb resolver).
pub type Resolver = ResolverFn;

// ── EngineApi (top-level) ──────────────────────────────────────

pub struct EngineApi {
    pub ctx: *mut u8,
    pub world: world::WorldApiResolved,
    pub client_world: client_world::ClientWorldApi,
    pub schedule: schedule::ScheduleApi,
    pub input: input::InputApi,
    pub physics: physics::PhysicsApiResolved,
    pub renderer: renderer::RendererApiResolved,
    pub ui: ui::UiApi,
    pub cmd: cmd::CmdApi,
    pub cvar: cvar::CvarApi,
    pub net: net::NetApi,
    pub host: host_access::HostAccessApi,
    pub resources: resources::ResourcesApi,
    pub audio: audio::AudioApi,
    pub materials: materials::MaterialsApi,

    pub frame: extern "C" fn(*mut u8, f32) -> i32,
    pub shutdown: extern "C" fn(*mut u8),
}

fn resolve_fn(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

unsafe impl Send for EngineApi {}
unsafe impl Sync for EngineApi {}

impl EngineApi {
    pub fn new(ctx: *mut u8, resolver: Resolver) -> Self {
        Self {
            ctx,
            world: world::WorldApiResolved::new(ctx, resolver),
            client_world: client_world::ClientWorldApi::new(ctx, resolver),
            schedule: schedule::ScheduleApi::new(ctx, resolver),
            input: input::InputApi::new(ctx, resolver),
            physics: physics::PhysicsApiResolved::new(ctx, resolver),
            renderer: renderer::RendererApiResolved::new(ctx, resolver),
            ui: ui::UiApi::new(ctx, resolver),
            cmd: cmd::CmdApi::new(ctx, resolver),
            cvar: cvar::CvarApi::new(ctx, resolver),
            net: net::NetApi::new(ctx, resolver),
            host: host_access::HostAccessApi::new(ctx, resolver),
            resources: resources::ResourcesApi::new(ctx, resolver),
            audio: audio::AudioApi::new(ctx, resolver),
            materials: materials::MaterialsApi::new(ctx, resolver),

            frame: unsafe { std::mem::transmute(resolve_fn(resolver, "v1:engine:frame")) },
            shutdown: unsafe { std::mem::transmute(resolve_fn(resolver, "v1:engine:shutdown")) },
        }
    }

    pub fn frame_api(&self, delta_time: f32) -> Result<(), String> {
        if (self.frame)(self.ctx, delta_time) == 0 {
            Ok(())
        } else {
            Err("frame failed".to_string())
        }
    }

    pub fn shutdown_api(&self) {
        (self.shutdown)(self.ctx);
    }
}
