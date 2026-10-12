pub mod app;
pub mod audio;
pub mod cmd;
pub mod cvar;
pub mod imgui;
pub mod input;
pub mod net;
pub mod physics;
pub mod renderer;
pub mod host_access;
pub mod resources;
pub mod scene;
pub mod schedule;
pub mod ui;
pub mod version;
pub mod methoddb;
pub mod world;
pub mod client_world;

use std::ffi::c_void;

pub type EntityId = u64;
pub type ComponentId = u64;

pub type Resolver = extern "C" fn(*const u8, usize) -> *const ();

// ── ServerInitData (mirrors host's repr(C) struct) ──────────

#[repr(C)]
pub struct ServerInitData {
    pub version: u32,
    pub ctx: *mut u8,
    pub resolver: Resolver,
}

// ── EngineApi (top-level) ──────────────────────────────────────

pub struct EngineApi {
    pub ctx: *mut u8,
    pub world: world::WorldApiResolved,
    pub client_world: client_world::ClientWorldApi,
    pub schedule: schedule::ScheduleApi,
    pub input: input::InputApi,
    pub imgui: imgui::ImGuiApi,
    pub physics: physics::PhysicsApiResolved,
    pub renderer: renderer::RendererApiResolved,
    pub scene: scene::SceneApi,
    pub ui: ui::UiApi,
    pub cmd: cmd::CmdApi,
    pub cvar: cvar::CvarApi,
    pub net: net::NetApi,
    pub host: host_access::HostAccessApi,
    pub resources: resources::ResourcesApi,
    pub audio: audio::AudioApi,

    pub initialize: extern "C" fn(*mut u8, *const u8, usize),
    pub frame: extern "C" fn(*mut u8, f32) -> i32,
    pub shutdown: extern "C" fn(*mut u8),
    pub with_world_mut: extern "C" fn(
        *mut u8,
        callback: extern "C" fn(*mut c_void, *mut c_void),
        user_data: *mut c_void,
        free_user_data: extern "C" fn(*mut c_void),
    ),
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
            imgui: imgui::ImGuiApi::new(ctx, resolver),
            physics: physics::PhysicsApiResolved::new(ctx, resolver),
            renderer: renderer::RendererApiResolved::new(ctx, resolver),
            scene: scene::SceneApi::new(ctx, resolver),
            ui: ui::UiApi::new(ctx, resolver),
            cmd: cmd::CmdApi::new(ctx, resolver),
            cvar: cvar::CvarApi::new(ctx, resolver),
            net: net::NetApi::new(ctx, resolver),
            host: host_access::HostAccessApi::new(ctx, resolver),
            resources: resources::ResourcesApi::new(ctx, resolver),
            audio: audio::AudioApi::new(ctx, resolver),

            initialize: unsafe { std::mem::transmute(resolve_fn(resolver, "v1:engine:initialize")) },
            frame: unsafe { std::mem::transmute(resolve_fn(resolver, "v1:engine:frame")) },
            shutdown: unsafe { std::mem::transmute(resolve_fn(resolver, "v1:engine:shutdown")) },
            with_world_mut: unsafe { std::mem::transmute(resolve_fn(resolver, "v1:engine:with_world_mut")) },
        }
    }

    pub fn initialize_api(&self, gamedir: &str) {
        let bytes = gamedir.as_bytes();
        (self.initialize)(self.ctx, bytes.as_ptr(), bytes.len());
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
