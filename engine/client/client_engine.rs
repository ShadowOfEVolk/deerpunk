use std::path::{Path, PathBuf};

use deerpunk_engine_common::input::Input;
use deerpunk_engine_common::resource::{path_to_file_uri, ResourceManager};
use deerpunk_engine_common::world::World;
use deerpunk_engine_renderer::gpu::{Renderer, ShaderSources};
use deerpunk_engine_renderer::snapshot::{RenderScene, RenderSnapshot, RenderTarget};
use deerpunk_ui::{UiDirAssets, UiRuntime};

use crate::app::App;
use crate::render_extractor::RenderExtractor;
use crate::ui_controller::UiController;

pub struct ClientEngine {
    pub(crate) app: App,
    renderer: Renderer,
    extractor: Option<RenderExtractor>,
    pub input: Input,
    ui: Option<UiRuntime>,
    pub ui_controller: UiController,
}

impl ClientEngine {
    pub fn new(resources: &mut ResourceManager) -> Self {
        let app = App::new("Deerpunk Engine", 1280, 720);

        let instance = wgpu::Instance::default();
        let surface = app.create_wgpu_surface(&instance);

        let mut renderer = Renderer::new(
            load_shader_sources(resources),
            &instance,
            Some(&surface),
        );

        let (w, h) = app.window_size();
        renderer.set_surface(surface, w, h);

        Self {
            app,
            renderer,
            extractor: None,
            input: Input::new(),
            ui: None,
            ui_controller: UiController::new(),
        }
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

    pub fn init_extractor(&mut self, world: &mut World) {
        self.extractor = Some(RenderExtractor::new(world));
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
        if !self.app.poll_events(&mut self.input) {
            return false;
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
        let snapshots = scene
            .map(|s| s.build_snapshots(aspect))
            .unwrap_or_default();
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

    pub fn extract(&self, world: &World) -> RenderScene {
        self.extractor
            .as_ref()
            .map(|e| e.extract(world))
            .unwrap_or_else(RenderScene::new)
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
    let ui = ui_handle
        .try_get()
        .unwrap_or_else(|| include_str!("../../shaders/ui.wgsl").to_string());

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
        ui,
    }
}
