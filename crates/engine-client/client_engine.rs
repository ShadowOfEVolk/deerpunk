use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deerpunk_engine_common::components::{CameraComponent, SoundEmitter, TransformComponent};
use deerpunk_engine_common::input::Input;
use deerpunk_engine_common::material::DrawMaterial;
use deerpunk_engine_common::material::{screen_dither_preprocessor_defines, ShaderPreprocessor};
use deerpunk_engine_common::network::sound_event::SoundPlayRpc;
use deerpunk_engine_common::network::ClientSessionPhase;
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::resource::{path_to_file_uri, ResourceManager, ShaderSource};
use deerpunk_engine_common::world::{QueryId, World};
use deerpunk_engine_renderer::debug_draw::DebugDraw;
use deerpunk_engine_renderer::material_store::MaterialStore;
use deerpunk_engine_renderer::renderer::{Renderer, ShaderSources};
use deerpunk_engine_renderer::shader_registry::ShaderRegistry;
use deerpunk_engine_renderer::snapshot::{RenderScene, RenderSnapshot, RenderTarget, SkyDraw};

use crate::camera_compose::pack_surface_camera;

/// Per-frame sky colors from gamelib (Hosek on CPU).
#[derive(Clone, Copy, Debug)]
pub struct GamelibSkyPayload {
    pub params: [f32; 32],
    pub horizon: [f32; 3],
    pub zenith: [f32; 3],
    pub sun_glow: f32,
}
use deerpunk_engine_renderer::CullScratch;
use deerpunk_ui::{UiDirAssets, UiRuntime};
use std::collections::HashMap;

pub use deerpunk_engine_renderer::{DebugColor, DebugDraw as DebugDrawBuf, DebugPrimitive};

use crate::app::App;
use crate::audio::{AudioBackend, AudioListener, FmodEventInfo, NullAudio};
use crate::bone_buffer_pool::{BoneBufferHandle, BoneBufferPool};
use crate::client_dll::ClientDll;
use crate::console::Console;
use crate::demo;
use crate::lan_browser::LanBrowser;
use crate::mesh_lifecycle::MeshGpuLifecycle;
use crate::network::ClientPrefabReplication;
use crate::render_extractor::RenderExtractor;
use crate::render_thread::PipelinedSubmit;
use crate::ui_controller::UiController;
use deerpunk_engine_server::ClientNet;

/// Client-owned camera stamp (reapplied after world replicate).
#[derive(Clone, Copy, Debug)]
pub struct LocalViewStamp {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub eye_height: f32,
    pub fov_y: f32,
}

#[derive(Clone, Copy)]
struct ServerLoopSound {
    net_id: u16,
    origin: [f32; 3],
}

#[derive(Default)]
struct UiLatch {
    wheel: f32,
    text: String,
    keys: Vec<(i32, bool)>,
    clicks: Vec<bool>,
}

pub struct ClientEngine {
    pub(crate) app: App,
    renderer: Renderer,
    /// Client ECS mirror (replicated entities, render hooks). One [`World::simulate`] per frame in host update.
    pub world: World,
    extractor: Option<RenderExtractor>,
    pub input: Input,
    /// Must be declared before [`Self::ui`]: Drop closes documents first,
    /// then RmlUi shuts down (declaration order).
    pub ui_controller: UiController,
    ui: Option<UiRuntime>,
    ui_fps: i32,
    ui_accum: f32,
    ui_screen: (u32, u32),
    ui_mouse: Option<(i32, i32)>,
    ui_has_frame: bool,
    draw_ui: bool,
    ui_latch: UiLatch,
    /// Plugged audio backend (`None` → disabled).
    audio: Option<Box<dyn AudioBackend>>,
    /// [`Host::resources`], set in [`set_audio_backend`]. Client tick thread only.
    audio_assets: *const ResourceManager,
    /// Client entity ids with a started attached (looping) event.
    audio_started: HashSet<u64>,
    /// Server `sound_id` → loop instance (FMOD key = sound_id).
    server_sound_loops: HashMap<u32, ServerLoopSound>,
    audio_query: Option<QueryId>,
    audio_scene_query: Option<QueryId>,
    /// Entity whose mesh was passed to the gamelib audio scene hook.
    audio_scene_entity: Option<u64>,
    /// Optional `client_audio_world_scene` from the client gamelib DLL.
    audio_world_scene_hook: Option<unsafe extern "C" fn(u64)>,
    /// Master volume (from `s_volume`), applied in the per-frame update.
    audio_volume: f32,
    /// Accumulated client time (seconds) for postfx / animated shaders.
    time: f32,
    prepare_systems:
        Vec<Box<dyn for<'a> FnMut(&mut deerpunk_engine_renderer::PrepareDrawList<'a>) + Send>>,
    /// [`SceneGeometry`] for `renderer.prepare`. Null outside that phase.
    prepare_scene: *mut deerpunk_engine_renderer::SceneGeometry,
    prepare_added: bool,
    /// Host/game fills each frame; copied into [`RenderSnapshot::debug_draw`].
    pending_debug_draw: DebugDraw,
    /// Shader registry — loaded .wgsl variants, cached by (path, defines).
    pub shader_registry: ShaderRegistry,
    /// Material store — loaded .mat assets, dedup by path/hash, GPU SSBO.
    pub material_store: MaterialStore,
    mesh_lifecycle: MeshGpuLifecycle,
    pub bone_buffers: BoneBufferPool,
    /// Shared fallback mesh (stable `content_id`, retained on GPU).
    fallback_mesh: Arc<Mesh>,
    /// In-game `~` console — keyboard routed to Nuklear before render.
    console_open: bool,
    pub dll: Option<ClientDll>,
    pub console: Console,
    pub rep: ClientPrefabReplication,
    /// Until `SVC::InitDone` (listen channel or UDP).
    pub join_phase: ClientSessionPhase,
    pub lan_browser: LanBrowser,
    pub demo_writer: Option<demo::DemoWriter>,
    pub demo_player: Option<demo::DemoPlayer>,
    /// Last albedo atlas slot from gamelib `flush_pending_albedos` FFI.
    pub last_atlas_slot: u32,
    /// `cl_cmdrate` throttle for outbound `UserCmd`.
    pub last_usercmd_send_secs: f32,
    pub usercmd_channel: crate::usercmd_channel::UsercmdChannel,
    pub local_sim: crate::local_sim::LocalSimOrg,
    pub network: ClientNet,
    /// Fullscreen sky (`gamedir/shaders/sky.wgsl`); colors from gamelib [`Self::gamelib_sky`].
    procedural_sky: bool,
    /// Latest Hosek payload from gamelib (`set_gamelib_sky` FFI).
    gamelib_sky: Option<GamelibSkyPayload>,
    cull_scratch: CullScratch,
    surface_snapshot: RenderSnapshot,
    /// Per-frame surface cameras (sorted by `order`) for compose.
    camera_build_scratch: Vec<deerpunk_engine_renderer::snapshot::CameraRenderSnapshot>,
    camera_compose_scratch: Vec<RenderSnapshot>,
    /// Gamelib overlays merged into frame debug draw (block outline, etc.).
    game_debug: DebugDraw,
    pipelined_submit: Option<PipelinedSubmit>,
    /// [`cl_render_thread`]: 0 = sync submit/present, 1 = async submit on `deerpunk-submit`.
    render_pipeline_mode: usize,
}

#[inline]
fn smoothstep_sky(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl ClientEngine {
    pub fn new(resources: &ResourceManager) -> Self {
        let app = App::new("Deerpunk Engine", 1280, 720);

        let instance = wgpu::Instance::default();
        let surface = app.create_wgpu_surface(&instance);

        let mut renderer =
            Renderer::new(load_shader_sources(resources, 0), &instance, Some(&surface));

        let (w, h) = app.framebuffer_size();
        deerpunk_engine_common::frame::set_frame_size(w, h);
        renderer.set_surface(surface, w, h);

        let shader_registry = ShaderRegistry::new();
        let material_store = MaterialStore::new();
        let fallback_mesh = Arc::new(Mesh::cube());
        renderer.mesh_retain(fallback_mesh.content_id(), 0);
        let mesh_lifecycle = MeshGpuLifecycle::new();
        let bone_buffers = BoneBufferPool::new();

        Self {
            app,
            renderer,
            world: World::new(),
            extractor: None,
            input: Input::new(),
            ui_controller: UiController::new(),
            ui: None,
            ui_fps: 30,
            ui_accum: 0.0,
            ui_screen: (0, 0),
            ui_mouse: None,
            ui_has_frame: false,
            draw_ui: true,
            ui_latch: UiLatch::default(),
            audio: None,
            audio_assets: std::ptr::null(),
            audio_started: HashSet::new(),
            server_sound_loops: HashMap::new(),
            audio_query: None,
            audio_scene_query: None,
            audio_scene_entity: None,
            audio_world_scene_hook: None,
            audio_volume: 1.0,
            time: 0.0,
            prepare_systems: Vec::new(),
            prepare_scene: std::ptr::null_mut(),
            prepare_added: false,
            pending_debug_draw: DebugDraw::new(),
            shader_registry,
            material_store,
            mesh_lifecycle,
            bone_buffers,
            fallback_mesh,
            console_open: false,
            dll: None,
            console: Console::new(),
            rep: ClientPrefabReplication::new(),
            join_phase: ClientSessionPhase::Idle,
            lan_browser: LanBrowser::default(),
            demo_writer: None,
            demo_player: None,
            last_atlas_slot: 0,
            last_usercmd_send_secs: -1.0,
            usercmd_channel: crate::usercmd_channel::UsercmdChannel::default(),
            local_sim: crate::local_sim::LocalSimOrg::default(),
            network: ClientNet::default(),
            procedural_sky: false,
            gamelib_sky: None,
            cull_scratch: CullScratch::new(),
            surface_snapshot: RenderSnapshot::new(
                deerpunk_engine_common::components::ColorLoad::Clear,
            ),
            camera_build_scratch: Vec::new(),
            camera_compose_scratch: Vec::new(),
            game_debug: DebugDraw::new(),
            pipelined_submit: None,
            render_pipeline_mode: 0,
        }
    }

    /// Gamelib Nuklear widgets (host GPU). Skipped for legacy client DLLs that rasterize in-process.
    pub fn gamelib_ui_build_frame(&self, _delta_time: f32) {
        let Some(ref dll) = self.dll else {
            return;
        };
        let Some((w, h, format)) = self.renderer.swapchain_layout() else {
            return;
        };
        let args = deerpunk_engine_ffi::GamelibUiBuildArgs {
            version: deerpunk_engine_ffi::GAMELIB_UI_API_VERSION,
            device: (&self.renderer.gpu.device as *const _ as *mut std::ffi::c_void),
            queue: (&self.renderer.gpu.queue as *const _ as *mut std::ffi::c_void),
            surface_format: crate::gamelib_ui::gamelib_ui_format_token(format),
            width: w as f32,
            height: h as f32,
            nk: crate::gamelib_ui::nuklear_draw_host(),
        };
        unsafe {
            let _ = dll.gamelib_ui_build(&args);
        }
    }

    fn render_surface_frame(
        &mut self,
        cameras: &[RenderSnapshot],
        post: &RenderSnapshot,
        geometry: Option<&deerpunk_engine_renderer::snapshot::SceneGeometry>,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        delta_time: f32,
    ) {
        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present.frame");
        let _ = delta_time;
        let gamelib_ui_render = self.dll.as_ref().and_then(|d| d.gamelib_ui_render_fn());

        if game_frame_fn.is_some() {
            if let Err(e) = self.renderer.render_to_surface_compose_with_game_frame(
                cameras,
                post,
                geometry,
                game_frame_fn,
                game_frame_ctx,
                gamelib_ui_render,
                &self.shader_registry,
            ) {
                self.handle_surface_frame_error(e);
            }
        } else if let Err(e) =
            self.renderer
                .render_to_surface_compose(cameras, post, geometry, &self.shader_registry)
        {
            self.handle_surface_frame_error(e);
        }
        deerpunk_engine_renderer::gamelib_profile::zone_end();
    }

    pub fn clear_game_debug(&mut self) {
        self.game_debug.clear();
    }

    pub fn game_debug_mut(&mut self) -> &mut DebugDraw {
        &mut self.game_debug
    }

    pub fn take_game_debug(&mut self) -> DebugDraw {
        std::mem::take(&mut self.game_debug)
    }

    pub fn set_procedural_sky(&mut self, on: bool) {
        self.procedural_sky = on;
    }

    /// Gamelib Hosek payload for the next scene extract (`fill_procedural_sky`).
    pub fn set_gamelib_sky(&mut self, payload: Option<GamelibSkyPayload>) {
        self.gamelib_sky = payload;
    }

    /// Same eye position the extractor already put on [`RenderScene::cameras`].
    fn sky_camera_pos(scene: &RenderScene) -> Option<[f32; 3]> {
        scene
            .cameras
            .iter()
            .find(|c| matches!(c.target, RenderTarget::Surface))
            .map(|c| c.position)
    }

    /// Fullscreen sky: sun + horizon from [`Renderer::scene_sun_dir`] /
    /// [`Renderer::scene_ambient`] (gamelib `set_scene_lighting`), zenith from
    /// [`Renderer::scene_sky_zenith`] (`set_sky_palette`). Optional Hosek params
    /// from `set_gamelib_sky` (exposure / day_fade in `params[30..32]`).
    fn fill_procedural_sky(&mut self, scene: &mut RenderScene) {
        if !self.procedural_sky {
            return;
        }
        let Some(camera_pos) = Self::sky_camera_pos(scene) else {
            return;
        };
        let sd = self.renderer.scene_sun_dir();
        let amb = self.renderer.scene_ambient();
        let zenith_default = self.renderer.scene_sky_zenith();
        let g = self.gamelib_sky;
        let mut io = crate::scene::SkySceneCompose {
            camera_pos,
            scene_sun: sd,
            ambient: amb,
            zenith_default,
            wrote: 0,
            clear: [0.0; 4],
            sun_dir: [0.0; 3],
            params: [0.0; 32],
            sky_color: [0.0; 3],
            zenith: [0.0; 3],
        };
        if let Some(ref dll) = self.dll {
            if let Some(compose) = dll.compose_sky_fn() {
                io.wrote = 0;
                unsafe {
                    compose(self as *mut Self as *mut u8, &mut io);
                }
                if io.wrote != 0 {
                    crate::scene::apply_compose(scene, &io);
                    return;
                }
            }
        }
        crate::scene::fill_procedural_sky_default(
            scene,
            camera_pos,
            sd,
            amb,
            zenith_default,
            g.map(|s| s.params),
            g.map(|s| s.horizon),
            g.map(|s| s.zenith),
            self.renderer.sky_debug_log(),
        );
    }

    pub fn set_console_open(&mut self, open: bool) {
        self.console_open = open;
    }

    pub fn mesh_lifecycle_adopt_chunk(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        entity: u64,
    ) {
        self.mesh_lifecycle
            .adopt_chunk(meshes, &self.world, &mut self.renderer, entity);
    }

    pub fn mesh_adopt_entity(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        entity: u64,
    ) {
        self.mesh_lifecycle
            .adopt_entity(meshes, &self.world, &mut self.renderer, entity);
    }

    pub fn mesh_release_entity(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        entity: u64,
    ) {
        self.mesh_lifecycle
            .release_entity(meshes, &self.world, &mut self.renderer, entity);
    }

    /// Release host mesh handles before Flecs deletes the entity (POD components, no `Drop`).
    pub fn drop_heap_mesh_components(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        entity: u64,
    ) {
        use deerpunk_engine_common::scene::{release_mesh3d_at, release_skinned_at};
        let mesh3d = self
            .world
            .component_init::<deerpunk_engine_common::components::mesh3d::Mesh3D>();
        let skinned = self
            .world
            .component_init::<deerpunk_engine_common::components::SkinnedMesh3D>();
        if self.world.has(entity, skinned) {
            release_skinned_at(&self.world, meshes, entity, skinned);
            self.world.remove(entity, skinned);
        }
        if self.world.has(entity, mesh3d) {
            release_mesh3d_at(&self.world, meshes, entity, mesh3d);
            self.world.remove(entity, mesh3d);
        }
    }

    pub fn mesh_lifecycle_reset(&mut self) {
        self.mesh_lifecycle.reset(&mut self.renderer);
        self.renderer
            .mesh_retain(self.fallback_mesh.content_id(), 0);
    }

    /// Client-side prediction/render state (materials, mesh GPU refs). ECS cleared separately.
    pub fn reset_render_resources(&mut self) {
        self.material_store.reset();
        self.mesh_lifecycle_reset();
    }

    /// Full client scene teardown: all ECS entities, stream state, mesh/material GPU caches.
    /// Apply one SVC replication packet (`StreamIn`, `NetDelta`, …).
    pub fn apply_replication_packet(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        packet: &[u8],
    ) -> bool {
        let rep = &mut self.rep as *mut ClientPrefabReplication;
        let engine = self as *mut ClientEngine;
        // SAFETY: `apply_packet` only uses disjoint fields of `ClientEngine` vs `rep`.
        unsafe { (*rep).apply_packet(&mut *engine, meshes, gamedir, resources, packet) }
    }

    /// Single client [`ecs_progress`] for this frame (after net apply, before gamelib `pre_render`).
    pub fn tick_world(&mut self, dt: f32) {
        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.ecs.simulate");
        self.world.simulate(dt.max(0.0));
        deerpunk_engine_renderer::gamelib_profile::zone_end();
    }

    pub fn refresh_stream_meshes(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
    ) {
        let entities = self.rep.replicated_entities();
        for entity in entities {
            let _ = deerpunk_engine_common::scene::resolve_mesh3d_uri(
                &self.world,
                meshes,
                gamedir,
                resources,
                entity,
            );
            let _ = crate::draw_material_resolve::resolve_draw_material_uri(
                self, resources, entity, false,
            );
            self.mesh_adopt_entity(meshes, entity);
        }
    }

    pub fn flush_client_scene(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
    ) {
        let ids: Vec<u64> = self
            .world
            .inspect_entities()
            .into_iter()
            .map(|e| e.id)
            .collect();
        log::info!("[client] flush scene — destroying {} entities", ids.len());
        for &id in &ids {
            self.mesh_release_entity(meshes, id);
            self.drop_heap_mesh_components(meshes, id);
        }
        for id in ids {
            self.world.destroy(id);
        }
        meshes.clear();
        self.world.simulate(0.0);
        self.rep.reset_stream_state();
        self.mesh_lifecycle.reset_tracking();
        self.reset_render_resources();
        self.pending_debug_draw = DebugDraw::new();
        self.audio_started.clear();
        self.server_sound_loops.clear();
        self.audio_scene_entity = None;
        self.usercmd_channel.reset();
        self.local_sim.clear();
        self.audio_query = None;
        self.audio_scene_query = None;
        if self.extractor.is_some() {
            self.init_extractor();
        }
    }

    pub fn fallback_mesh(&self) -> &Arc<Mesh> {
        &self.fallback_mesh
    }

    /// Mutable debug line buffer for the next surface render.
    pub fn debug_draw(&mut self) -> &mut DebugDraw {
        &mut self.pending_debug_draw
    }

    pub fn advance_time(&mut self, dt: f32) {
        self.time += dt.max(0.0);
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn set_relative_mouse(&mut self, on: bool) {
        self.app.set_relative_mouse(on);
    }

    /// Poll OS events into [`Self::input`]. Returns `false` on quit.
    pub fn poll_events(&mut self) -> bool {
        if !self.app.poll_events(&mut self.input) {
            return false;
        }
        if let Some((w, h)) = self.app.take_pending_resize() {
            self.resize_window(w, h);
        }
        true
    }

    /// Swapchain + HDR/depth/postfx + logical frame size (see [`Renderer::resize_framebuffer`]).
    pub fn resize_window(&mut self, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);
        deerpunk_engine_common::frame::set_frame_size(w, h);
        self.renderer.resize_framebuffer(w, h);
        if let Some(ui) = self.ui.as_ref() {
            ui.resize(w, h);
        }
        log::info!("[client] window resized to {w}x{h}");
    }

    fn handle_surface_frame_error(&mut self, err: deerpunk_engine_renderer::FrameError) {
        use deerpunk_engine_renderer::FrameError;
        match err {
            FrameError::SurfaceOutdated | FrameError::SurfaceLost => {
                let (w, h) = self.app.framebuffer_size();
                self.resize_window(w, h);
            }
            FrameError::SurfaceOccluded | FrameError::SurfaceTimeout => {}
            FrameError::SurfaceValidation => {
                log::warn!("[client] surface validation error — reconfiguring");
                let (w, h) = self.app.framebuffer_size();
                self.resize_window(w, h);
            }
        }
    }

    /// Apply gameinfo window title + icon (call after gamedir / gameinfo are known).
    pub fn apply_window_identity(&mut self, title: &str, icon_path: Option<&Path>) {
        self.app.set_title(title);
        if let Some(path) = icon_path {
            self.app.set_icon_from_path(path);
        }
    }

    /// Install an audio backend and init it against `gamedir` (banks in `{gamedir}/sound`).
    pub fn set_audio_backend(
        &mut self,
        mut backend: Box<dyn AudioBackend>,
        gamedir: &std::path::Path,
        assets: &ResourceManager,
    ) {
        self.audio_assets = assets;
        let ctx = crate::audio::AudioInitContext { gamedir };
        let ok = backend.init(&ctx);
        log::info!(
            "[client] audio backend init: {}",
            if ok { "ok" } else { "FAILED (disabled)" }
        );
        self.audio = if ok { Some(backend) } else { None };
    }

    /// Master volume for the next audio update (`s_volume`).
    pub fn set_audio_volume(&mut self, volume: f32) {
        self.audio_volume = volume.clamp(0.0, 1.0);
    }

    /// One-shot Studio event at the listener (UI / non-positional).
    pub fn audio_play_2d(
        &mut self,
        event: &str,
        volume: f32,
        params: &[(&str, f32)],
        instance_id: u32,
    ) {
        if let Some(a) = self.audio.as_mut() {
            with_audio_assets(self.audio_assets, |assets| {
                a.play_event_2d(assets, event, volume, params, instance_id);
            });
        }
    }

    /// One-shot Studio event at a world position.
    pub fn audio_play_3d(
        &mut self,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    ) {
        if let Some(a) = self.audio.as_mut() {
            with_audio_assets(self.audio_assets, |assets| {
                a.play_event_3d(assets, event, origin, volume, params);
            });
        }
    }

    /// Start a looping event attached to a client-world entity.
    pub fn audio_start_attached(
        &mut self,
        entity: u64,
        event: &str,
        origin: [f32; 3],
        volume: f32,
    ) {
        if let Some(a) = self.audio.as_mut() {
            with_audio_assets(self.audio_assets, |assets| {
                a.start_attached(assets, entity, event, origin, volume);
            });
            self.audio_started.insert(entity);
        }
    }

    /// Stop the attached event for a client-world entity.
    pub fn audio_stop_attached(&mut self, entity: u64) {
        if let Some(a) = self.audio.as_mut() {
            a.stop_attached(entity);
        }
        self.audio_started.remove(&entity);
    }

    pub fn audio_apply_play(&mut self, rpc: &SoundPlayRpc) {
        let params: Vec<(&str, f32)> = rpc.params.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        if rpc.is_2d() {
            self.audio_play_2d(&rpc.event, rpc.volume, &params, rpc.sound_id);
            return;
        }
        if rpc.is_loop() {
            let key = rpc.sound_id as u64;
            if let Some(a) = self.audio.as_mut() {
                with_audio_assets(self.audio_assets, |assets| {
                    a.start_attached(assets, key, &rpc.event, rpc.origin, rpc.volume);
                });
            }
            self.server_sound_loops.insert(
                rpc.sound_id,
                ServerLoopSound {
                    net_id: rpc.net_id,
                    origin: rpc.origin,
                },
            );
            return;
        }
        self.audio_play_3d(&rpc.event, rpc.origin, rpc.volume, &params);
    }

    pub fn audio_apply_stop(&mut self, sound_id: u32) {
        let key = sound_id as u64;
        if let Some(a) = self.audio.as_mut() {
            a.stop_instance(sound_id);
            a.stop_attached(key);
        }
        self.server_sound_loops.remove(&sound_id);
    }

    pub fn server_loops_with_net_id(&self) -> Vec<(u32, u16)> {
        self.server_sound_loops
            .iter()
            .filter(|(_, s)| s.net_id != 0)
            .map(|(&id, s)| (id, s.net_id))
            .collect()
    }

    pub fn set_server_loop_origin(&mut self, sound_id: u32, origin: [f32; 3]) {
        if let Some(s) = self.server_sound_loops.get_mut(&sound_id) {
            s.origin = origin;
        }
    }

    /// Enumerate Studio events from loaded FMOD banks.
    pub fn audio_merge_sound_events(
        &mut self,
        registry: &deerpunk_engine_common::sound_event::SoundEventRegistry,
    ) {
        if let Some(a) = self.audio.as_mut() {
            a.merge_sound_events(registry);
        }
    }

    pub fn audio_list_events(&self) -> Vec<FmodEventInfo> {
        self.audio
            .as_ref()
            .map(|a| a.list_events())
            .unwrap_or_default()
    }

    /// Listener + attached-sound positions for entities with `SoundEmitter`.
    fn audio_tick(&mut self, listener: AudioListener) {
        if self.audio.is_none() {
            return;
        }

        // Auto-start autoplaying emitters, collect fresh positions for started ones.
        let t_id = self.world.component_init::<TransformComponent>();
        let se_id = self.world.component_init::<SoundEmitter>();
        let q = match self.audio_query {
            Some(q) => q,
            None => {
                let q = self.world.query_prepare_ids(&[t_id, se_id]);
                self.audio_query = Some(q);
                q
            }
        };

        let mut attached: Vec<(u64, [f32; 3])> = Vec::new();
        let mut to_start: Vec<(u64, [f32; 3], f32, String)> = Vec::new();
        if let Some(batches) = self.world.query_batches(q) {
            for batch in batches {
                let count = batch.count;
                let ents = unsafe { std::slice::from_raw_parts(batch.entities, count) };
                let transforms = unsafe {
                    std::slice::from_raw_parts(batch.columns[0] as *const TransformComponent, count)
                };
                let emitters = unsafe {
                    std::slice::from_raw_parts(batch.columns[1] as *const SoundEmitter, count)
                };
                for (i, e) in ents.iter().enumerate() {
                    let emitter = &emitters[i];
                    let event = emitter.event_str();
                    if event.is_empty() {
                        continue;
                    }
                    let pos = transforms[i].position.to_array();
                    if self.audio_started.contains(e) {
                        attached.push((*e, pos));
                    } else if emitter.autoplay {
                        to_start.push((*e, pos, emitter.volume, event.to_string()));
                    }
                }
            }
        }

        // Install the Steam Audio world scene when a (new) BSP mesh appears.
        self.audio_scene_tick();

        let assets_ptr = self.audio_assets;
        let Some(backend) = self.audio.as_mut() else {
            return;
        };
        with_audio_assets(assets_ptr, |assets| {
            for (e, pos, volume, event) in to_start {
                log::info!("[client] audio: autoplay event '{event}' on entity {e:#x}");
                backend.start_attached(assets, e, &event, pos, volume);
                self.audio_started.insert(e);
                attached.push((e, pos));
            }
        });

        for (&sid, st) in &self.server_sound_loops {
            attached.push((sid as u64, st.origin));
        }

        backend.update(&listener, &attached, self.audio_volume);
    }

    /// Notifies gamelib when a world mesh entity appears (gamelib installs audio via API).
    fn audio_scene_tick(&mut self) {
        let hook = self.audio_world_scene_hook;
        if hook.is_none() {
            return;
        }
        let mesh_id = self
            .world
            .component_init::<deerpunk_engine_common::components::mesh3d::Mesh3D>();
        let ws_id = self
            .world
            .component_init::<deerpunk_engine_common::components::Worldspawn>();
        let q = match self.audio_scene_query {
            Some(q) => q,
            None => {
                let q = self.world.query_prepare_ids(&[ws_id, mesh_id]);
                self.audio_scene_query = Some(q);
                q
            }
        };
        let mut installed = self.audio_scene_entity;
        let mut found = false;
        if let Some(batches) = self.world.query_batches(q) {
            for batch in batches {
                let ents = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let meshes = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[1]
                            as *const deerpunk_engine_common::components::mesh3d::Mesh3D,
                        batch.count,
                    )
                };
                for (i, e) in ents.iter().enumerate() {
                    if !meshes[i].mesh.is_valid() {
                        continue;
                    }
                    found = true;
                    if self.audio_scene_entity != Some(*e) {
                        log::info!("[client] audio: world scene hook (entity {e:#x})");
                        unsafe {
                            hook.unwrap()(*e);
                        }
                        self.audio_scene_entity = Some(*e);
                    }
                    installed = Some(*e);
                    break;
                }
            }
        }
        self.world.query_release(q);
        self.audio_scene_entity = if found { installed } else { None };
    }

    pub fn set_audio_world_scene_hook(&mut self, hook: Option<unsafe extern "C" fn(u64)>) {
        self.audio_world_scene_hook = hook;
        self.audio_scene_entity = None;
    }

    /// Gamelib `v1:audio:set_world_mesh` — mesh from client world entity.
    pub fn install_audio_world_mesh(
        &mut self,
        meshes: &deerpunk_engine_common::resource::MeshRegistry,
        entity: u64,
    ) -> bool {
        let mesh_id = self
            .world
            .component_init::<deerpunk_engine_common::components::Mesh3D>();
        let ptr = self.world.get_raw(entity, mesh_id);
        if ptr.is_null() {
            return false;
        }
        let mesh3d =
            unsafe { &*(ptr as *const deerpunk_engine_common::components::mesh3d::Mesh3D) };
        let Some(mesh) = meshes.get(mesh3d.mesh) else {
            return false;
        };
        let Some(backend) = self.audio.as_mut() else {
            return false;
        };
        backend.set_world_scene(mesh.as_ref());
        true
    }

    /// Init RmlUi from `{ui_root}` (usually `{gamedir}/ui`). Widgets via [`UiController`].
    pub fn init_ui(&mut self, ui_root: impl Into<PathBuf>) {
        let ui_root = ui_root.into();
        log::info!("[client] init_ui from {}", ui_root.display());
        let (w, h) = self.app.framebuffer_size();
        let assets = Box::new(UiDirAssets::new(ui_root));
        let ui = UiRuntime::new(assets, (w, h), "main");
        if !ui.is_ready() {
            log::error!("[client] UiRuntime failed — no UI this session");
            return;
        }
        self.ui = Some(ui);
    }

    /// Close all documents, then `Rml::Shutdown`. Idempotent.
    pub fn shutdown_ui(&mut self) {
        self.ui_controller.clear();
        if let Some(ui) = self.ui.take() {
            drop(ui);
            log::info!("[client] RmlUi shutdown OK");
        }
    }

    pub fn init_extractor(&mut self) {
        self.extractor = Some(RenderExtractor::new(&mut self.world));
    }

    pub fn extract_scene(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
    ) -> RenderScene {
        self.extract_scene_view(meshes, None, true, true)
    }

    /// Extract with optional view entity (server `SetView` net pawn).
    pub fn extract_scene_view(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        view_entity: Option<u64>,
        early_cull: bool,
        draw_world: bool,
    ) -> RenderScene {
        let mut extract_gpu = deerpunk_engine_renderer::SceneExtractGpuStats::default();

        self.renderer.bind_mesh_registry(meshes);
        let fallback = self.fallback_mesh.clone();
        let aspect = deerpunk_engine_common::frame::frame_width() as f32
            / deerpunk_engine_common::frame::frame_height().max(1) as f32;
        if let Some(ext) = self.extractor.as_ref() {
            let _z = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.sync_world");
            let sync_start = std::time::Instant::now();
            let sync_stats = ext.sync_world_transforms(&mut self.world);
            extract_gpu.sync_world_ms = sync_start.elapsed().as_secs_f32() * 1000.0;
            extract_gpu.sync_world_unchanged = sync_stats.unchanged;
        }
        let mut scene = self
            .extractor
            .as_mut()
            .map(|e| {
                e.extract(
                    &self.world,
                    meshes,
                    view_entity,
                    &mut self.material_store,
                    &mut self.renderer.material_allocator,
                    &self.renderer.gpu.device,
                    &self.renderer.gpu.queue,
                    &fallback,
                    &mut extract_gpu,
                    aspect,
                    early_cull,
                    draw_world,
                )
            })
            .unwrap_or_else(RenderScene::new);
        let post_start = std::time::Instant::now();
        scene.game_time = self.time;
        {
            let _z = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.bones");
            self.fill_bone_palettes(&mut scene);
        }
        {
            let _z = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.sky");
            self.fill_procedural_sky(&mut scene);
        }
        extract_gpu.post_ms = post_start.elapsed().as_secs_f32() * 1000.0;
        self.renderer.note_extract_gpu_stats(extract_gpu);
        scene
    }

    /// Register a system that runs after extract and before camera snapshots.
    ///
    /// [`PrepareDrawList::push_mesh`] appends a draw. `batchable` draws join the
    /// mesh pass groups; the batch plan itself is still built by the renderer.
    pub fn add_prepare_system<F>(&mut self, system: F)
    where
        F: for<'a> FnMut(&mut deerpunk_engine_renderer::PrepareDrawList<'a>) + Send + 'static,
    {
        self.prepare_systems.push(Box::new(system));
    }

    /// Append prepare-system draws into `scene` and refresh the cull serial.
    pub fn run_prepare(&mut self, scene: &mut deerpunk_engine_renderer::RenderScene) {
        deerpunk_engine_renderer::run_prepare_systems(scene, &mut self.prepare_systems);
    }

    /// Draw list for the upcoming `renderer.prepare` stage. The pointer stays
    /// valid until [`Self::finish_mesh_submit`].
    pub fn begin_mesh_submit(&mut self, geometry: *mut deerpunk_engine_renderer::SceneGeometry) {
        self.prepare_scene = geometry;
        self.prepare_added = false;
    }

    pub fn finish_mesh_submit(&mut self, scene: &mut deerpunk_engine_renderer::RenderScene) {
        self.prepare_scene = std::ptr::null_mut();
        if self.prepare_added {
            scene.geometry.refresh_cull_serial();
            self.prepare_added = false;
        }
    }

    /// Append one static mesh to the prepare draw list. `batchable` so it joins
    /// the material/shader group. Position / rotation (xyzw) / scale are world TRS.
    pub fn submit_mesh(
        &mut self,
        meshes: &deerpunk_engine_common::resource::MeshRegistry,
        entity: u64,
        mesh: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        material: deerpunk_engine_common::material::MaterialHandle,
        uniform_aux: u32,
        layer_mask: u32,
        position: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
        first_index: u32,
        index_count: u32,
        face: u8,
    ) -> bool {
        if self.prepare_scene.is_null() || !mesh.is_valid() || layer_mask == 0 || index_count == 0 {
            return false;
        }
        let Some(cpu) = meshes.get(mesh) else {
            return false;
        };
        let Some(entry) = self.material_store.get(material) else {
            return false;
        };
        if entry.shader == 0 {
            return false;
        }
        let texture = entry.albedo;
        let normal = entry.normal;
        let orm = entry.orm;
        let shader_id = entry.shader;
        let material_id = self.material_store.resolve_gpu(
            material,
            &mut self.renderer.material_allocator,
            &self.renderer.gpu.device,
            &self.renderer.gpu.queue,
        );
        let model = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::from_array(scale),
            glam::Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
            glam::Vec3::from_array(position),
        );
        let local = deerpunk_engine_renderer::Aabb::from_min_max(
            glam::Vec3::from_array(cpu.bounds_min),
            glam::Vec3::from_array(cpu.bounds_max),
        );
        let world_bounds = local.transform(model);
        let draw = deerpunk_engine_renderer::snapshot::MeshDraw {
            mesh,
            model_mat: model.to_cols_array_2d(),
            layer_mask,
            batchable: true,
            material_id,
            texture,
            normal,
            orm,
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0; 4],
            shader_id,
            lightmap_gpu: 0,
            first_index,
            index_count,
            face,
            world_bounds,
            entity_id: entity,
            uniform_aux,
        };
        unsafe {
            (*self.prepare_scene).mesh_draws.push(draw);
        }
        self.prepare_added = true;
        true
    }

    pub fn bone_buffer_alloc(&mut self, bone_count: usize) -> BoneBufferHandle {
        self.bone_buffers.alloc(bone_count)
    }

    pub fn bone_buffer_free(&mut self, handle: BoneBufferHandle) {
        self.bone_buffers.free(handle);
    }

    pub fn bone_buffer_write(&mut self, handle: BoneBufferHandle, mats: &[glam::Mat4]) -> bool {
        self.bone_buffers.write(handle, mats)
    }

    fn fill_bone_palettes(&mut self, scene: &mut RenderScene) {
        use deerpunk_engine_renderer::snapshot::BonePaletteUpload;
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        scene.geometry.bone_palettes.clear();
        for draw in &scene.geometry.skinned_mesh_draws {
            if draw.bone_buffer == 0 || !seen.insert(draw.bone_buffer) {
                continue;
            }
            let handle = BoneBufferHandle(draw.bone_buffer);
            if let Some(mats) = self.bone_buffers.matrices(handle) {
                let matrices = mats.iter().map(|m| m.to_cols_array_2d()).collect();
                scene.geometry.bone_palettes.push(BonePaletteUpload {
                    handle: draw.bone_buffer,
                    matrices,
                });
            }
        }
    }

    pub fn set_draw_ui(&mut self, draw: bool) {
        self.draw_ui = draw;
    }

    pub fn ui_runtime(&self) -> Option<&UiRuntime> {
        self.ui.as_ref()
    }

    pub fn ui_load(&mut self, id: &str) -> Result<(), String> {
        let Some(ui) = self.ui.as_ref() else {
            return Err("ui runtime not ready".into());
        };
        // Load goes through RmlUi file interface → UiDirAssets (asset root).
        // Later: ResourceManager `ui://…` can feed the same path.
        self.ui_controller.load(ui, id)
    }

    pub fn ui_unload(&mut self, id: &str) -> Result<(), String> {
        self.ui_controller.unload(id)
    }

    pub fn ui_show(&mut self, id: &str) -> Result<(), String> {
        self.ui_controller.show(id)
    }

    pub fn ui_hide(&mut self, id: &str) -> Result<(), String> {
        self.ui_controller.hide(id)
    }

    pub fn ui_lua_eval(&mut self, code: &str) -> Result<(), String> {
        deerpunk_ui::lua_eval(code);
        Ok(())
    }

    pub fn ui_data_model_set_bool(
        &mut self,
        document: &str,
        field: &str,
        value: bool,
    ) -> Result<(), String> {
        if self.ui.is_none() {
            return Err("ui runtime not ready".into());
        }
        self.ui_controller
            .data_model_set(document, field, deerpunk_ui::DataModelValue::Bool(value))
    }

    pub fn ui_data_model_set_number(
        &mut self,
        document: &str,
        field: &str,
        value: f64,
    ) -> Result<(), String> {
        if self.ui.is_none() {
            return Err("ui runtime not ready".into());
        }
        self.ui_controller.data_model_set(
            document,
            field,
            deerpunk_ui::DataModelValue::Number(value),
        )
    }

    pub fn ui_data_model_set_string(
        &mut self,
        document: &str,
        field: &str,
        value: &str,
    ) -> Result<(), String> {
        if self.ui.is_none() {
            return Err("ui runtime not ready".into());
        }
        self.ui_controller.data_model_set(
            document,
            field,
            deerpunk_ui::DataModelValue::String(value.to_string()),
        )
    }

    pub fn set_ui_fps(&mut self, fps: i32) {
        self.ui_fps = fps;
    }

    fn ui_due(&mut self, dt: f32) -> bool {
        if !self.ui_has_frame || self.ui_fps <= 0 {
            self.ui_accum = 0.0;
            return true;
        }
        self.ui_accum += dt;
        let step = 1.0 / self.ui_fps.max(1) as f32;
        if self.ui_accum >= step {
            self.ui_accum = (self.ui_accum - step).min(step);
            true
        } else {
            false
        }
    }

    fn latch_ui_input(&mut self, wheel: f32) {
        if self.input.mouse_button_pressed(0) {
            self.ui_latch.clicks.push(true);
        }
        if self.input.mouse_button_released(0) {
            self.ui_latch.clicks.push(false);
        }
        if self.console_open {
            return;
        }
        self.ui_latch.wheel += wheel;
        self.ui_latch.keys.extend(self.input.drain_ui_keys());
        let mut buf = [0u8; 128];
        let n = self.input.take_text(&mut buf);
        if n > 0 {
            if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                self.ui_latch.text.push_str(s);
            }
        }
    }

    fn flush_ui(&mut self, w: u32, h: u32) {
        let latch = std::mem::take(&mut self.ui_latch);
        let resize = self.ui_screen != (w, h);
        if resize {
            self.ui_screen = (w, h);
        }
        let (mx, my) = self.input.mouse_position();
        let mouse = (mx as i32, my as i32);
        let mouse_moved = self.ui_mouse != Some(mouse);
        if mouse_moved {
            self.ui_mouse = Some(mouse);
        }
        let frame = {
            let Some(ui) = self.ui.as_ref() else {
                return;
            };
            if resize {
                ui.resize(w, h);
            }
            if mouse_moved {
                ui.mouse_move(mouse.0, mouse.1);
            }
            for down in latch.clicks {
                ui.mouse_button(0, down);
            }
            if latch.wheel != 0.0 {
                ui.mouse_wheel(latch.wheel);
            }
            if !latch.keys.is_empty() {
                crate::ui_rml_input::feed_rml_keys(ui, latch.keys.into_iter());
            }
            if !latch.text.is_empty() {
                crate::ui_rml_input::feed_rml_text(ui, &latch.text);
            }
            ui.frame()
        };
        if let Some(frame) = frame {
            static LOG_ONCE: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !LOG_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::info!(
                    "[client] first UiFrame: batches={} verts={} idx={} tex_uploads={}",
                    frame.batches.len(),
                    frame.vertices.len(),
                    frame.indices.len(),
                    frame.textures.len()
                );
            }
            self.ui_has_frame = true;
            self.renderer.set_ui_frame(Some(frame));
        }
    }

    pub fn tick(
        &mut self,
        scene: Option<&RenderScene>,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        delta_time: f32,
    ) -> bool {
        self.tick_inner(scene, game_frame_fn, game_frame_ctx, delta_time, true)
    }

    /// Render a frame. If `poll` is false, caller must have already called [`Self::poll_events`].
    pub fn tick_inner(
        &mut self,
        scene: Option<&RenderScene>,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        delta_time: f32,
        poll: bool,
    ) -> bool {
        if poll && !self.app.poll_events(&mut self.input) {
            return false;
        }

        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.input");
        let wheel = self.input.take_mouse_wheel();
        {
            let (mx, my) = self.input.mouse_position();
            let b = self.input.mouse_buttons();
            let mut text = String::new();
            let mut keys = Vec::new();
            if self.console_open {
                self.console.capture_keys(&self.input);
                let mut buf = [0u8; 512];
                let n = self.input.take_text(&mut buf);
                if n > 0 {
                    text = String::from_utf8_lossy(&buf[..n]).into_owned();
                }
                let map = [
                    (40u16, deerpunk_engine_renderer::host_ui::UiKey::enter()),
                    (42, deerpunk_engine_renderer::host_ui::UiKey::backspace()),
                    (76, deerpunk_engine_renderer::host_ui::UiKey::del()),
                    (80, deerpunk_engine_renderer::host_ui::UiKey::left()),
                    (79, deerpunk_engine_renderer::host_ui::UiKey::right()),
                ];
                for (sc, key) in map {
                    keys.push((key, self.input.is_down(sc)));
                }
                keys.push((
                    deerpunk_engine_renderer::host_ui::UiKey::ctrl(),
                    self.input.is_down(224) || self.input.is_down(228),
                ));
                keys.push((
                    deerpunk_engine_renderer::host_ui::UiKey::shift(),
                    self.input.is_down(225) || self.input.is_down(229),
                ));
                self.input.clear_keys();
            }
            self.renderer
                .set_host_ui_input(deerpunk_engine_renderer::host_ui::UiFrameInput {
                    mouse: [mx, my],
                    buttons: [b & 1 != 0, b & 2 != 0, b & 4 != 0],
                    wheel: if self.console_open { wheel } else { 0.0 },
                    text,
                    keys,
                });
        }
        deerpunk_engine_renderer::gamelib_profile::zone_end();

        let prep_start = std::time::Instant::now();
        let (w, h) = self.app.framebuffer_size();
        let ui_start = std::time::Instant::now();
        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.rmlui");
        if !self.draw_ui {
            self.ui_has_frame = false;
            self.renderer.set_ui_frame(None);
        } else if self.ui.is_some() {
            if self.ui_controller.any_visible() {
                self.latch_ui_input(wheel);
                if self.ui_due(delta_time) {
                    self.flush_ui(w, h);
                }
            } else {
                self.ui_has_frame = false;
                self.renderer.set_ui_frame(None);
            }
        } else {
            static WARN_NO_UI: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !WARN_NO_UI.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!("[client] ui runtime is None — init_ui failed?");
            }
        }
        self.input.end_frame_input();
        deerpunk_engine_renderer::gamelib_profile::zone_end();
        let ui_ms = ui_start.elapsed().as_secs_f32() * 1000.0;

        // No cameras (e.g. menu-only) still need a surface pass so the UI overlay runs.
        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.snapshots");
        let snap_start = std::time::Instant::now();
        let aspect = w as f32 / h as f32;
        if let Some(s) = scene.as_ref() {
            s.build_snapshots_into(
                aspect,
                &mut self.cull_scratch,
                &mut self.camera_build_scratch,
            );
        } else {
            self.camera_build_scratch.clear();
        }
        let build_snap_ms = snap_start.elapsed().as_secs_f32() * 1000.0;
        let frustum_ms = self.cull_scratch.last_frustum_ms;
        let assemble_ms = self.cull_scratch.last_assemble_ms;
        deerpunk_engine_renderer::gamelib_profile::zone_end();

        let mut surface_indices: Vec<usize> = self
            .camera_build_scratch
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.target, RenderTarget::Surface))
            .map(|(i, _)| i)
            .collect();
        surface_indices.sort_by_key(|&i| {
            let s = &self.camera_build_scratch[i];
            (s.order, s.camera_pos[0].to_bits())
        });

        // Audio: listener from lowest-order surface camera.
        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.audio");
        let audio_start = std::time::Instant::now();
        {
            let listener = surface_indices
                .first()
                .map(|&i| listener_from_view(self.camera_build_scratch[i].view))
                .unwrap_or_else(|| {
                    self.camera_build_scratch
                        .first()
                        .map(|s| listener_from_view(s.view))
                        .unwrap_or_default()
                });
            self.audio_tick(listener);
        }
        let audio_ms = audio_start.elapsed().as_secs_f32() * 1000.0;
        deerpunk_engine_renderer::gamelib_profile::zone_end();

        self.renderer
            .note_prep_breakdown(ui_ms, build_snap_ms, audio_ms);

        let mut gather_ms = 0.0f32;
        let _ = prep_start;

        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present");
        let mut rendered_surface = false;
        if !surface_indices.is_empty() {
            self.camera_compose_scratch.clear();
            deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present.gather");
            if let Some(sc) = scene {
                for (i, &cam_idx) in surface_indices.iter().enumerate() {
                    let cam = &self.camera_build_scratch[cam_idx];
                    let gather_start = std::time::Instant::now();
                    let mut snap = RenderSnapshot::new(cam.color_load);
                    pack_surface_camera(sc, cam, &mut snap);
                    gather_ms += gather_start.elapsed().as_secs_f32() * 1000.0;
                    if i == 0 {
                        snap.debug_draw = std::mem::take(&mut self.pending_debug_draw.primitives);
                    }
                    self.camera_compose_scratch.push(snap);
                }
            }
            deerpunk_engine_renderer::gamelib_profile::zone_end();
            let post_idx = 0usize;
            deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present.clone");
            let cameras = self.camera_compose_scratch.clone();
            let post_snap = cameras[post_idx].clone();
            let geometry_owned = scene.as_ref().map(|s| s.geometry.clone());
            deerpunk_engine_renderer::gamelib_profile::zone_end();
            self.render_surface_frame(
                &cameras,
                &post_snap,
                geometry_owned.as_ref(),
                game_frame_fn,
                game_frame_ctx,
                delta_time,
            );
            deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present.drop");
            drop(cameras);
            drop(post_snap);
            drop(geometry_owned);
            deerpunk_engine_renderer::gamelib_profile::zone_end();
            rendered_surface = true;
        }
        self.renderer
            .note_cull_detail(frustum_ms, assemble_ms, gather_ms);
        deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present.scratch");
        for snap in self.camera_build_scratch.drain(..) {
            if !snap.frustum_culled {
                continue;
            }
            if snap.visible_mesh.capacity() > self.cull_scratch.visible_mesh.capacity() {
                self.cull_scratch.visible_mesh = snap.visible_mesh;
            }
            if snap.visible_skinned.capacity() > self.cull_scratch.visible_skinned.capacity() {
                self.cull_scratch.visible_skinned = snap.visible_skinned;
            }
        }
        deerpunk_engine_renderer::gamelib_profile::zone_end();
        if !rendered_surface {
            let snapshot =
                RenderSnapshot::new(deerpunk_engine_common::components::ColorLoad::Clear);
            self.render_surface_frame(
                std::slice::from_ref(&snapshot),
                &snapshot,
                None,
                game_frame_fn,
                game_frame_ctx,
                delta_time,
            );
        }
        if self.render_pipeline_mode >= 1 {
            deerpunk_engine_renderer::gamelib_profile::zone_begin("client.tick.present.kick");
            self.pipelined_submit_kick();
            deerpunk_engine_renderer::gamelib_profile::zone_end();
        }
        deerpunk_engine_renderer::gamelib_profile::zone_end();
        true
    }

    pub fn set_render_pipeline(&mut self, mode: usize) {
        let mode = mode.min(1);
        self.render_pipeline_mode = mode;
        self.renderer.set_defer_surface_submit(mode >= 1);
        if mode >= 1 {
            if self.pipelined_submit.is_none() {
                let queue = self.renderer.gpu.queue.clone();
                self.pipelined_submit = Some(PipelinedSubmit::new(queue));
            }
        } else {
            self.shutdown_pipelined_submit();
        }
    }

    pub fn set_pipelined_submit(&mut self, enabled: bool) {
        self.set_render_pipeline(if enabled { 1 } else { 0 });
    }

    pub fn pipelined_submit_wait(&mut self) {
        let Some(ps) = self.pipelined_submit.as_mut() else {
            return;
        };
        ps.wait_and_complete(&mut self.renderer);
    }

    pub fn pipelined_submit_kick(&mut self) {
        if !self.renderer.defer_surface_submit() {
            return;
        }
        let queue = self.renderer.gpu.queue.clone();
        let ps = self
            .pipelined_submit
            .get_or_insert_with(|| PipelinedSubmit::new(queue));
        ps.kick(&mut self.renderer);
    }

    pub fn shutdown_pipelined_submit(&mut self) {
        self.pipelined_submit_wait();
        if let Some(ps) = self.pipelined_submit.take() {
            ps.shutdown();
        }
        self.renderer.set_defer_surface_submit(false);
    }

    pub fn shutdown_render_pipeline(&mut self) {
        self.shutdown_pipelined_submit();
        self.render_pipeline_mode = 0;
    }

    /// Drawable size in physical pixels (render resolution).
    pub fn window_size(&self) -> (u32, u32) {
        self.app.framebuffer_size()
    }

    pub fn set_text_input_active(&mut self, active: bool) {
        self.app.set_text_input_active(active);
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn material_handle_for_path(
        &self,
        path: &str,
    ) -> Option<deerpunk_engine_common::material::MaterialHandle> {
        self.material_store.handle_for_path(path)
    }

    /// Load a `.mat` file or URI-backed material into the material store.
    pub fn load_material_resolved(
        &mut self,
        path: &str,
        source: &str,
        resources: &ResourceManager,
    ) -> Result<deerpunk_engine_common::material::MaterialHandle, String> {
        let shader_registry = &mut self.shader_registry;
        let handle = self.material_store.load_with_gpu(
            path,
            source,
            resources,
            &mut self.renderer,
            shader_registry,
        )?;
        Ok(handle)
    }

    /// Create a GPU material from an in-memory [`MaterialAsset`] (e.g. from RM).
    pub fn load_material_from_asset(
        &mut self,
        path: &str,
        asset: &deerpunk_engine_common::material::MaterialAsset,
        resources: &ResourceManager,
    ) -> Result<deerpunk_engine_common::material::MaterialHandle, String> {
        let shader_registry = &mut self.shader_registry;
        let handle = self.material_store.create_from_asset_with_gpu(
            path,
            asset,
            resources,
            &mut self.renderer,
            shader_registry,
        )?;
        Ok(handle)
    }

    /// Re-read WGSL from gamedir: built-in passes (sky, postfx, …) + all [`ShaderRegistry`] variants.
    pub fn reload_shaders(
        &mut self,
        resources: &ResourceManager,
        screen_dither_mode: i32,
    ) -> Result<(), String> {
        let shader_dir = std::env::var("SHADER_DIR").unwrap_or_else(|_| "shaders".to_string());
        let shader_dir = Path::new(&shader_dir);

        for name in [
            "sprite",
            "mesh_skinned",
            "terrain_splat",
            "sky",
            "cloud",
            "debug",
            "tonemap",
            "luminance",
            "bloom_threshold",
            "bloom_blur",
            "bloom_composite",
            "grain",
            "ui",
        ] {
            let path = shader_dir.join(format!("{name}.wgsl"));
            resources.evict(&path_to_file_uri(&path));
        }

        let variants = self.shader_registry.cached_variants();
        let variant_count = variants.len();
        for (path, _) in &variants {
            resources.evict(path);
        }

        let content_root = resources.base_dir();
        for (path, defines_key) in variants {
            let defines: Vec<String> = if defines_key.is_empty() {
                Vec::new()
            } else {
                defines_key.split(',').map(|s| s.to_string()).collect()
            };
            let handle = resources.load::<ShaderSource>(&path);
            let source = handle
                .try_get()
                .ok_or_else(|| format!("reload: failed to load {path}"))?;
            self.shader_registry
                .reload(&path, &defines, &source.wgsl, &content_root)?;
        }

        self.material_store.bump_shader_generations();
        let sources = load_shader_sources(resources, screen_dither_mode);
        self.renderer.reload_shader_sources(sources);
        log::info!("[shaders] hot-reload OK ({variant_count} material shader variants)");
        Ok(())
    }
}

/// Listener from a world→view matrix (position = view origin).
fn listener_from_view(view: glam::Mat4) -> AudioListener {
    let inv = view.inverse();
    // Matches `CameraDefinition::view_matrix` (look_to_rh, forward +Y, world Z-up).
    let mut forward = inv.transform_vector3(glam::Vec3::NEG_Z);
    let mut up = inv.transform_vector3(glam::Vec3::Y);
    if up.length_squared() < 1e-8 {
        up = glam::Vec3::Z;
    }
    forward = forward.normalize_or_zero();
    up = up.normalize_or_zero();
    let pos = inv.transform_point3(glam::Vec3::ZERO);
    AudioListener {
        position: pos.to_array(),
        forward: forward.to_array(),
        up: up.to_array(),
    }
}

/// `ClientEngine::audio_assets` — Host-owned [`ResourceManager`], client tick thread only.
fn with_audio_assets<R>(
    ptr: *const ResourceManager,
    f: impl FnOnce(&ResourceManager) -> R,
) -> Option<R> {
    if ptr.is_null() {
        None
    } else {
        Some(f(unsafe { &*ptr }))
    }
}

fn load_wgsl_required(
    rm: &ResourceManager,
    shader_dir: &Path,
    name: &str,
    defines: &[String],
) -> String {
    let path = shader_dir.join(format!("{name}.wgsl"));
    let uri = path_to_file_uri(&path);
    let handle = rm.load::<ShaderSource>(&uri);
    let source = handle
        .try_get()
        .unwrap_or_else(|| {
            panic!(
                "Failed to load shader via ResourceManager: {uri} (asset={:?})",
                handle.id()
            )
        })
        .wgsl;
    // Strip `#@shader` / `#@prop` decorators (and apply `#ifdef` handling) before
    // handing the source to wgpu — wgpu would otherwise choke on the `#@` lines,
    // which are not valid WGSL. Files without decorators pass through unchanged.
    let base = path.parent();
    match ShaderPreprocessor::process_at(&source, defines, base) {
        Ok(processed) => processed.wgsl,
        Err(err) => panic!("Failed to preprocess shader {uri}: {err}"),
    }
}

/// Load WGSL sources through [`ResourceManager`] (`file://…/*.wgsl`).
fn load_shader_sources(rm: &ResourceManager, screen_dither_mode: i32) -> ShaderSources {
    let shader_dir = std::env::var("SHADER_DIR").unwrap_or_else(|_| "shaders".to_string());
    let shader_dir = Path::new(&shader_dir);
    let dither_defines = screen_dither_preprocessor_defines(screen_dither_mode);
    let no_defines: &[String] = &[];

    let ui = load_wgsl_required(rm, shader_dir, "ui", no_defines);

    ShaderSources {
        sprite: load_wgsl_required(rm, shader_dir, "sprite", no_defines),
        mesh_skinned: load_wgsl_required(rm, shader_dir, "mesh_skinned", no_defines),
        terrain_splat: load_wgsl_required(rm, shader_dir, "terrain_splat", no_defines),
        sky: load_wgsl_required(rm, shader_dir, "sky", no_defines),
        cloud: load_wgsl_required(rm, shader_dir, "cloud", no_defines),
        debug: load_wgsl_required(rm, shader_dir, "debug", no_defines),
        tonemap: load_wgsl_required(rm, shader_dir, "tonemap", no_defines),
        luminance: load_wgsl_required(rm, shader_dir, "luminance", no_defines),
        bloom_threshold: load_wgsl_required(rm, shader_dir, "bloom_threshold", no_defines),
        bloom_blur: load_wgsl_required(rm, shader_dir, "bloom_blur", no_defines),
        bloom_composite: load_wgsl_required(rm, shader_dir, "bloom_composite", no_defines),
        grain: load_wgsl_required(rm, shader_dir, "grain", no_defines),
        ui,
    }
}
