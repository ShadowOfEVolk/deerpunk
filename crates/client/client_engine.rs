use std::collections::HashSet;
use std::path::{Path, PathBuf};

use deerpunk_engine_common::network::sound_event::SoundPlayRpc;
use std::collections::HashMap;
use deerpunk_engine_common::components::{SoundEmitter, TransformComponent};
use deerpunk_engine_common::input::Input;
use deerpunk_engine_common::resource::{path_to_file_uri, ResourceManager};
use deerpunk_engine_common::world::{QueryId, World};
use deerpunk_engine_renderer::debug_draw::DebugDraw;
use deerpunk_engine_renderer::gpu::{Renderer, ShaderSources};
use deerpunk_engine_renderer::snapshot::{RenderScene, RenderSnapshot, RenderTarget};
use deerpunk_ui::{UiDirAssets, UiRuntime};

pub use deerpunk_engine_renderer::{DebugColor, DebugDraw as DebugDrawBuf, DebugPrimitive};

use crate::app::App;
use crate::audio::{AudioBackend, AudioListener, FmodEventInfo, NullAudio};
use crate::render_extractor::RenderExtractor;
use crate::ui_controller::UiController;

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
    /// Client entity ids with a started attached (looping) event.
    audio_started: HashSet<u64>,
    /// Server `sound_id` → loop instance (FMOD key = sound_id).
    server_sound_loops: HashMap<u32, ServerLoopSound>,
    audio_query: Option<QueryId>,
    /// Entity whose BSP mesh was installed as the audio simulation scene.
    audio_scene_entity: Option<u64>,
    /// Master volume (from `s_volume`), applied in the per-frame update.
    audio_volume: f32,
    /// Accumulated client time (seconds) for postfx / animated shaders.
    time: f32,
    /// Host/game fills each frame; copied into [`RenderSnapshot::debug_draw`].
    pending_debug_draw: DebugDraw,
}

impl ClientEngine {
    pub fn new(resources: &mut ResourceManager) -> Self {
        let app = App::new("Deerpunk Engine", 1280, 720);

        let instance = wgpu::Instance::default();
        let surface = app.create_wgpu_surface(&instance);

        let mut renderer = Renderer::new(load_shader_sources(resources), &instance, Some(&surface));

        let (w, h) = app.window_size();
        renderer.set_surface(surface, w, h);

        Self {
            app,
            renderer,
            world: World::new(),
            extractor: None,
            input: Input::new(),
            ui_controller: UiController::new(),
            ui: None,
            audio: None,
            audio_started: HashSet::new(),
            server_sound_loops: HashMap::new(),
            audio_query: None,
            audio_scene_entity: None,
            audio_volume: 1.0,
            time: 0.0,
            pending_debug_draw: DebugDraw::new(),
        }
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
    pub fn set_audio_backend(&mut self, mut backend: Box<dyn AudioBackend>, gamedir: &Path) {
        let ok = backend.init(gamedir);
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
    pub fn audio_play_2d(&mut self, event: &str, volume: f32, params: &[(&str, f32)]) {
        if let Some(a) = self.audio.as_mut() {
            a.play_event_2d(event, volume, params);
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
            a.play_event_3d(event, origin, volume, params);
        }
    }

    /// Start a looping event attached to a client-world entity.
    pub fn audio_start_attached(&mut self, entity: u64, event: &str, origin: [f32; 3], volume: f32) {
        if let Some(a) = self.audio.as_mut() {
            a.start_attached(entity, event, origin, volume);
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
        let params: Vec<(&str, f32)> = rpc
            .params
            .iter()
            .map(|(n, v)| (n.as_str(), *v))
            .collect();
        if rpc.is_2d() {
            self.audio_play_2d(&rpc.event, rpc.volume, &params);
            return;
        }
        if rpc.is_loop() {
            let key = rpc.sound_id as u64;
            if let Some(a) = self.audio.as_mut() {
                a.start_attached(key, &rpc.event, rpc.origin, rpc.volume);
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

        let Some(backend) = self.audio.as_mut() else {
            return;
        };

        for (e, pos, volume, event) in to_start {
            log::info!("[client] audio: autoplay event '{event}' on entity {e:#x}");
            backend.start_attached(e, &event, pos, volume);
            self.audio_started.insert(e);
            attached.push((e, pos));
        }

        for (&sid, st) in &self.server_sound_loops {
            attached.push((sid as u64, st.origin));
        }

        backend.update(&listener, &attached, self.audio_volume);
    }

    /// Installs the world BSP mesh as the audio simulation scene (once per map).
    fn audio_scene_tick(&mut self) {
        let Some(backend) = self.audio.as_mut() else {
            return;
        };
        let dbg_id = self.world.component_init::<deerpunk_engine_common::bsp_debug::BspWorldDebug>();
        let mesh_id = self.world.component_init::<deerpunk_engine_common::components::mesh3d::Mesh3D>();
        let q = self.world.query_prepare_ids(&[dbg_id, mesh_id]);
        let mut installed = self.audio_scene_entity;
        let mut found = false;
        if let Some(batches) = self.world.query_batches(q) {
            for batch in batches {
                let ents = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let meshes = unsafe {
                    std::slice::from_raw_parts(batch.columns[1] as *const deerpunk_engine_common::components::mesh3d::Mesh3D, batch.count)
                };
                for (i, e) in ents.iter().enumerate() {
                    found = true;
                    if let Some(mesh) = &meshes[i].mesh {
                        if self.audio_scene_entity != Some(*e) {
                            // Clone the debug meta once — it may be large.
                            let debug = unsafe {
                                (*(batch.columns[0] as *const deerpunk_engine_common::bsp_debug::BspWorldDebug)).clone()
                            };
                            log::info!("[client] audio: world scene install (entity {e:#x})");
                            backend.set_world_scene(mesh, &debug);
                        }
                        installed = Some(*e);
                        break;
                    }
                }
            }
        }
        self.world.query_release(q);
        // World cleared / map changed — allow reinstall.
        self.audio_scene_entity = if found { installed } else { None };
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

    pub fn extract_scene(&self) -> RenderScene {
        self.extract_scene_view(None)
    }

    /// Extract with optional view entity (server `SetView` net pawn).
    pub fn extract_scene_view(&self, view_entity: Option<u64>) -> RenderScene {
        let mut scene = self
            .extractor
            .as_ref()
            .map(|e| e.extract(&self.world, view_entity))
            .unwrap_or_else(RenderScene::new);
        scene.game_time = self.time;
        scene
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

        // Feed OS mouse into ImGui (console / debug windows).
        {
            let (mx, my) = self.input.mouse_position();
            let b = self.input.mouse_buttons();
            self.renderer.set_imgui_input(
                [mx, my],
                [b & 1 != 0, b & 2 != 0, b & 4 != 0, false, false],
            );
        }

        let (w, h) = self.app.window_size();
        if let Some(ui) = self.ui.as_ref() {
            ui.resize(w, h);
            let (mx, my) = self.input.mouse_position();
            ui.mouse_move(mx as i32, my as i32);
            if self.input.mouse_button_pressed(0) {
                ui.mouse_button(0, true);
            }
            if self.input.mouse_button_released(0) {
                ui.mouse_button(0, false);
            }
            let wheel = self.input.take_mouse_wheel();
            if wheel != 0.0 {
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
            static WARN_NO_UI: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !WARN_NO_UI.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!("[client] ui runtime is None — init_ui failed?");
            }
        }
        self.input.end_frame_input();

        // No cameras (e.g. menu-only) still need a surface pass so the UI overlay runs.
        let aspect = w as f32 / h as f32;
        let snapshots = scene.map(|s| s.build_snapshots(aspect)).unwrap_or_default();

        // Audio: listener from the primary view, then per-frame backend update.
        {
            let listener = snapshots
                .first()
                .map(|s| listener_from_view(s.view))
                .unwrap_or_default();
            self.audio_tick(listener);
        }

        let mut rendered_surface = false;
        let mut first = true;
        for snap in &snapshots {
            if !matches!(snap.target, RenderTarget::Surface) {
                continue;
            }
            let mut snapshot = RenderSnapshot::new(snap.clear);
            snapshot.view_proj = snap.view_proj.to_cols_array_2d();
            snapshot.camera_pos = snap
                .view
                .inverse()
                .transform_point3(glam::Vec3::ZERO)
                .to_array();
            snapshot.game_time = snap.game_time;
            for md in &snap.visible_meshes {
                snapshot.mesh_draws.push(md.clone());
            }
            snapshot.point_lights = snap.point_lights.clone();
            snapshot.sky = snap.sky;
            if first {
                snapshot.debug_draw = std::mem::take(&mut self.pending_debug_draw.primitives);
            }
            if first && game_frame_fn.is_some() {
                let _ = self.renderer.render_to_surface_with_game_frame(
                    &snapshot,
                    game_frame_fn,
                    game_frame_ctx,
                );
            } else {
                let _ = self.renderer.render_to_surface(&snapshot);
            }
            first = false;
            rendered_surface = true;
        }
        if !rendered_surface {
            let snapshot = RenderSnapshot::new([0.08, 0.09, 0.12, 1.0]);
            if game_frame_fn.is_some() {
                let _ = self.renderer.render_to_surface_with_game_frame(
                    &snapshot,
                    game_frame_fn,
                    game_frame_ctx,
                );
            } else {
                let _ = self.renderer.render_to_surface(&snapshot);
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
}

/// Listener from a world→view matrix (position = view origin).
fn listener_from_view(view: glam::Mat4) -> AudioListener {
    let inv = view.inverse();
    // Camera looks down −Z in view space; map axes back to world space.
    let forward = inv.transform_vector3(glam::Vec3::NEG_Z);
    let up = inv.transform_vector3(glam::Vec3::Y);
    let pos = inv.transform_point3(glam::Vec3::ZERO);
    AudioListener {
        position: pos.to_array(),
        forward: forward.to_array(),
        up: up.to_array(),
    }
}

fn load_wgsl_required(rm: &mut ResourceManager, shader_dir: &Path, name: &str) -> String {
    let path = shader_dir.join(format!("{name}.wgsl"));
    let uri = path_to_file_uri(&path);
    let handle = rm.load::<String>(&uri);
    handle.try_get().unwrap_or_else(|| {
        panic!(
            "Failed to load shader via ResourceManager: {uri} (asset={:?})",
            handle.id()
        )
    })
}

/// Load WGSL sources through [`ResourceManager`] (`file://…/*.wgsl`).
fn load_shader_sources(rm: &mut ResourceManager) -> ShaderSources {
    let shader_dir = std::env::var("SHADER_DIR").unwrap_or_else(|_| "shaders".to_string());
    let shader_dir = Path::new(&shader_dir);

    let ui_path = shader_dir.join("ui.wgsl");
    let ui_uri = path_to_file_uri(&ui_path);
    let ui_handle = rm.load::<String>(&ui_uri);
    let ui = ui_handle.try_get().unwrap();

    ShaderSources {
        sprite: load_wgsl_required(rm, shader_dir, "sprite"),
        mesh: load_wgsl_required(rm, shader_dir, "mesh"),
        mesh_array: load_wgsl_required(rm, shader_dir, "mesh_array"),
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
