use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deerpunk_engine_common::components::{CameraComponent, SoundEmitter, TransformComponent};
use deerpunk_engine_common::material::DrawMaterial;
use deerpunk_engine_common::goldsrc_lightstyle::{LightstyleTable, MAX_LIGHTSTYLES};
use deerpunk_engine_common::network::ClientSessionPhase;
use deerpunk_engine_common::input::Input;
use deerpunk_engine_common::material::ShaderPreprocessor;
use deerpunk_engine_common::network::sound_event::SoundPlayRpc;
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::resource::{path_to_file_uri, ResourceManager, ShaderSource};
use deerpunk_engine_common::world::{QueryId, World};
use deerpunk_engine_renderer::debug_draw::DebugDraw;
use deerpunk_engine_renderer::gpu::{Renderer, ShaderSources};
use deerpunk_engine_renderer::material_store::MaterialStore;
use deerpunk_engine_renderer::shader_registry::ShaderRegistry;
use deerpunk_engine_renderer::snapshot::{RenderScene, RenderSnapshot, RenderTarget, SkyDraw};

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
use crate::client_dll::ClientDll;
use crate::console::Console;
use crate::demo;
use crate::lan_browser::LanBrowser;
use deerpunk_engine_server::ClientNet;
use crate::network::ClientPrefabReplication;
use crate::audio::{AudioBackend, AudioListener, FmodEventInfo, NullAudio};
use crate::bone_buffer_pool::{BoneBufferHandle, BoneBufferPool};
use crate::mesh_lifecycle::MeshGpuLifecycle;
use crate::render_extractor::RenderExtractor;
use crate::ui_controller::UiController;

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

pub struct ClientEngine {
    pub(crate) app: App,
    renderer: Renderer,
    /// Client-side ECS (replicated renderables / linked cameras). Not simulated.
    pub world: World,
    extractor: Option<RenderExtractor>,
    pub input: Input,
    /// Must be declared before [`Self::ui`]: Drop closes documents first,
    /// then RmlUi shuts down (declaration order).
    pub ui_controller: UiController,
    ui: Option<UiRuntime>,
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
    /// In-game `~` console — keyboard routed to ImGui before render.
    console_open: bool,
    pub dll: Option<ClientDll>,
    pub console: Console,
    pub rep: ClientPrefabReplication,
    /// Until `SVC::InitDone` (listen channel or UDP).
    pub join_phase: ClientSessionPhase,
    pub lan_browser: LanBrowser,
    pub lightstyles: LightstyleTable,
    pub last_ls_patterns: Vec<String>,
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
    /// Gamelib overlays merged into frame debug draw (block outline, etc.).
    game_debug: DebugDraw,
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

        let mut renderer = Renderer::new(load_shader_sources(resources), &instance, Some(&surface));

        let (w, h) = app.window_size();
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
            lightstyles: LightstyleTable::hl_defaults(),
            last_ls_patterns: vec![String::new(); MAX_LIGHTSTYLES],
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
            surface_snapshot: RenderSnapshot::new([0.08, 0.09, 0.12, 1.0]),
            game_debug: DebugDraw::new(),
        }
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

    /// Animate HL lightstyle table and push scales to the renderer.
    pub fn tick_lightstyles(&mut self, time_secs: f32) {
        self.lightstyles.tick(time_secs);
        let scales = self.lightstyles.scales_array();
        self.renderer_mut().set_lightstyles(&scales);
    }
    pub fn set_console_open(&mut self, open: bool) {
        self.console_open = open;
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
        self.renderer.mesh_retain(self.fallback_mesh.content_id(), 0);
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
                self,
                resources,
                entity,
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
        self.app.poll_events(&mut self.input)
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
    pub fn audio_play_2d(&mut self, event: &str, volume: f32, params: &[(&str, f32)], instance_id: u32) {
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
        let (w, h) = self.app.window_size();
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
        self.extract_scene_view(meshes, None, true)
    }

    /// Extract with optional view entity (server `SetView` net pawn).
    pub fn extract_scene_view(
        &mut self,
        meshes: &mut deerpunk_engine_common::resource::MeshRegistry,
        view_entity: Option<u64>,
        early_cull: bool,
    ) -> RenderScene {
        let mut extract_gpu = deerpunk_engine_renderer::SceneExtractGpuStats::default();

        let fallback = self.fallback_mesh.clone();
        let aspect = deerpunk_engine_common::frame::frame_width() as f32
            / deerpunk_engine_common::frame::frame_height().max(1) as f32;
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
                    &self.renderer.device,
                    &self.renderer.queue,
                    &fallback,
                    &mut extract_gpu,
                    aspect,
                    early_cull,
                )
            })
            .unwrap_or_else(RenderScene::new);
        let post_start = std::time::Instant::now();
        scene.game_time = self.time;
        self.fill_bone_palettes(&mut scene);
        self.fill_procedural_sky(&mut scene);
        extract_gpu.post_ms = post_start.elapsed().as_secs_f32() * 1000.0;

        self.renderer.note_extract_gpu_stats(extract_gpu);
        scene
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
        self.ui_controller.data_model_set(
            document,
            field,
            deerpunk_ui::DataModelValue::Bool(value),
        )
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

    pub fn tick(
        &mut self,
        scene: Option<&RenderScene>,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
    ) -> bool {
        self.tick_inner(scene, game_frame_fn, game_frame_ctx, true)
    }

    /// Render a frame. If `poll` is false, caller must have already called [`Self::poll_events`].
    pub fn tick_inner(
        &mut self,
        scene: Option<&RenderScene>,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        poll: bool,
    ) -> bool {
        if poll && !self.app.poll_events(&mut self.input) {
            return false;
        }

        if self.console_open {
            unsafe {
                crate::imgui_console_input::feed(&mut self.input, &self.app);
            }
            self.input.clear_keys();
        }

        let wheel = self.input.take_mouse_wheel();

        // Feed OS mouse into ImGui (console / debug windows).
        {
            let (mx, my) = self.input.mouse_position();
            let b = self.input.mouse_buttons();
            let (wheel_y, wheel_h) = if self.console_open {
                (wheel, 0.0)
            } else {
                (0.0, 0.0)
            };
            self.renderer.set_imgui_input(
                [mx, my],
                [b & 1 != 0, b & 2 != 0, b & 4 != 0, false, false],
                wheel_y,
                wheel_h,
            );
        }

        let prep_start = std::time::Instant::now();
        let (w, h) = self.app.window_size();
        let ui_start = std::time::Instant::now();
        if let Some(ui) = self.ui.as_ref() {
            if self.ui_controller.any_visible() {
                ui.resize(w, h);
                let (mx, my) = self.input.mouse_position();
                ui.mouse_move(mx as i32, my as i32);
                if self.input.mouse_button_pressed(0) {
                    ui.mouse_button(0, true);
                }
                if self.input.mouse_button_released(0) {
                    ui.mouse_button(0, false);
                }
                if !self.console_open && wheel != 0.0 {
                    ui.mouse_wheel(wheel);
                }
                if let Some(frame) = ui.frame() {
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
                    self.renderer.set_ui_frame(Some(frame));
                }
            } else {
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
        let ui_ms = ui_start.elapsed().as_secs_f32() * 1000.0;

        // No cameras (e.g. menu-only) still need a surface pass so the UI overlay runs.
        let snap_start = std::time::Instant::now();
        let aspect = w as f32 / h as f32;
        let mut snapshots = scene
            .map(|s| s.build_snapshots(aspect, &mut self.cull_scratch))
            .unwrap_or_default();
        let build_snap_ms = snap_start.elapsed().as_secs_f32() * 1000.0;
        let frustum_ms = self.cull_scratch.last_frustum_ms;
        let assemble_ms = self.cull_scratch.last_assemble_ms;

        // Audio: listener from the primary view, then per-frame backend update.
        let audio_start = std::time::Instant::now();
        {
            let listener = snapshots
                .first()
                .map(|s| listener_from_view(s.view))
                .unwrap_or_default();
            self.audio_tick(listener);
        }
        let audio_ms = audio_start.elapsed().as_secs_f32() * 1000.0;

        self.renderer
            .note_prep_breakdown(ui_ms, build_snap_ms, audio_ms);

        let mut gather_ms = 0.0f32;
        let _ = prep_start;

        let mut rendered_surface = false;
        let mut first = true;
        for snap in &snapshots {
            if !matches!(snap.target, RenderTarget::Surface) {
                continue;
            }
            let snapshot = &mut self.surface_snapshot;
            snapshot.clear = snap.clear;
            snapshot.view_proj = snap.view_proj.to_cols_array_2d();
            snapshot.inv_view_proj = snap.inv_view_proj.to_cols_array_2d();
            snapshot.inv_view_proj_sky = snap.inv_view_proj_sky.to_cols_array_2d();
            snapshot.camera_pos = snap.camera_pos;
            snapshot.game_time = snap.game_time;
            snapshot.instances.clear();
            snapshot.hud.clear();
            snapshot.terrain_splat_draws.clear();
            if let Some(sc) = scene {
                let gather_start = std::time::Instant::now();
                sc.gather_visible_draws(snap, &mut snapshot.mesh_draws, &mut snapshot.skinned_mesh_draws);
                gather_ms += gather_start.elapsed().as_secs_f32() * 1000.0;
                snapshot.viewmodel_mesh_draws.clone_from(&sc.geometry.viewmodel_mesh_draws);
                snapshot.viewmodel_view_proj = sc
                    .geometry
                    .viewmodel_pass
                    .as_ref()
                    .map(|p| p.view_proj.to_cols_array_2d());
                snapshot.viewmodel_camera_pos = sc
                    .geometry
                    .viewmodel_pass
                    .as_ref()
                    .map(|p| p.camera_pos)
                    .unwrap_or(snap.camera_pos);
            } else {
                snapshot.mesh_draws.clear();
                snapshot.skinned_mesh_draws.clear();
                snapshot.viewmodel_mesh_draws.clear();
                snapshot.viewmodel_view_proj = None;
            }
            snapshot.bone_palettes.clone_from(&snap.bone_palettes);
            snapshot.point_lights.clone_from(&snap.point_lights);
            snapshot.cull_stats = snap.cull_stats;
            snapshot.frustum_culled = snap.frustum_culled;
            snapshot.sky = snap.sky;
            snapshot.storm = None;
            if first {
                snapshot.debug_draw = std::mem::take(&mut self.pending_debug_draw.primitives);
            } else {
                snapshot.debug_draw.clear();
            }
            if first && game_frame_fn.is_some() {
                let _ = self.renderer.render_to_surface_with_game_frame(
                    snapshot,
                    game_frame_fn,
                    game_frame_ctx,
                    &self.shader_registry,
                );
            } else {
                let _ = self
                    .renderer
                    .render_to_surface(snapshot, &self.shader_registry);
            }
            first = false;
            rendered_surface = true;
            break;
        }
        self.renderer
            .note_cull_detail(frustum_ms, assemble_ms, gather_ms);
        for snap in snapshots.drain(..) {
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
        if !rendered_surface {
            let snapshot = RenderSnapshot::new([0.08, 0.09, 0.12, 1.0]);
            if game_frame_fn.is_some() {
                let _ = self.renderer.render_to_surface_with_game_frame(
                    &snapshot,
                    game_frame_fn,
                    game_frame_ctx,
                    &self.shader_registry,
                );
            } else {
                let _ = self
                    .renderer
                    .render_to_surface(&snapshot, &self.shader_registry);
            }
        }
        true
    }

    pub fn window_size(&self) -> (u32, u32) {
        self.app.window_size()
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

    pub fn material_handle_for_path(&self, path: &str) -> Option<deerpunk_engine_common::material::MaterialHandle> {
        self.material_store.handle_for_path(path)
    }

    /// Load a `.mat` file or URI-backed material into the material store.
    pub fn load_material_resolved(
        &mut self,
        path: &str,
        source: &str,
        resources: &ResourceManager,
    ) -> Result<deerpunk_engine_common::material::MaterialHandle, String> {
        let renderer = &mut self.renderer;
        let material_store = &mut self.material_store;
        let shader_registry = &mut self.shader_registry;
        let mut resolver =
            deerpunk_engine_renderer::material_store::GpuTextureResolver::new(resources, renderer);
        let handle = material_store.load(path, source, shader_registry, &mut resolver)?;
        if let Ok(asset) = deerpunk_engine_common::material::MaterialAsset::from_json(source) {
            material_store.attach_lightmaps_from_asset(handle, &asset, resources, renderer);
        }
        Ok(handle)
    }

    /// Create a GPU material from an in-memory [`MaterialAsset`] (e.g. from RM).
    pub fn load_material_from_asset(
        &mut self,
        path: &str,
        asset: &deerpunk_engine_common::material::MaterialAsset,
        resources: &ResourceManager,
    ) -> Result<deerpunk_engine_common::material::MaterialHandle, String> {
        let renderer = &mut self.renderer;
        let material_store = &mut self.material_store;
        let shader_registry = &mut self.shader_registry;
        let mut resolver =
            deerpunk_engine_renderer::material_store::GpuTextureResolver::new(resources, renderer);
        let handle =
            material_store.create_from_asset(path, asset, shader_registry, &mut resolver)?;
        material_store.attach_lightmaps_from_asset(handle, asset, resources, renderer);
        Ok(handle)
    }

    /// Re-read WGSL from gamedir: built-in passes (sky, postfx, …) + all [`ShaderRegistry`] variants.
    pub fn reload_shaders(&mut self, resources: &ResourceManager) -> Result<(), String> {
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
            "storm",
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
        let sources = load_shader_sources(resources);
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
fn with_audio_assets<R>(ptr: *const ResourceManager, f: impl FnOnce(&ResourceManager) -> R) -> Option<R> {
    if ptr.is_null() {
        None
    } else {
        Some(f(unsafe { &*ptr }))
    }
}

fn load_wgsl_required(rm: &ResourceManager, shader_dir: &Path, name: &str) -> String {
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
    match ShaderPreprocessor::process_at(&source, &[], base) {
        Ok(processed) => processed.wgsl,
        Err(err) => panic!("Failed to preprocess shader {uri}: {err}"),
    }
}

/// Load WGSL sources through [`ResourceManager`] (`file://…/*.wgsl`).
fn load_shader_sources(rm: &ResourceManager) -> ShaderSources {
    let shader_dir = std::env::var("SHADER_DIR").unwrap_or_else(|_| "shaders".to_string());
    let shader_dir = Path::new(&shader_dir);

    let ui = load_wgsl_required(rm, shader_dir, "ui");

    ShaderSources {
        sprite: load_wgsl_required(rm, shader_dir, "sprite"),
        mesh_skinned: load_wgsl_required(rm, shader_dir, "mesh_skinned"),
        terrain_splat: load_wgsl_required(rm, shader_dir, "terrain_splat"),
        sky: load_wgsl_required(rm, shader_dir, "sky"),
        cloud: load_wgsl_required(rm, shader_dir, "cloud"),
        debug: load_wgsl_required(rm, shader_dir, "debug"),
        tonemap: load_wgsl_required(rm, shader_dir, "tonemap"),
        luminance: load_wgsl_required(rm, shader_dir, "luminance"),
        storm: load_wgsl_required(rm, shader_dir, "storm"),
        bloom_threshold: load_wgsl_required(rm, shader_dir, "bloom_threshold"),
        bloom_blur: load_wgsl_required(rm, shader_dir, "bloom_blur"),
        bloom_composite: load_wgsl_required(rm, shader_dir, "bloom_composite"),
        grain: load_wgsl_required(rm, shader_dir, "grain"),
        ui,
    }
}
