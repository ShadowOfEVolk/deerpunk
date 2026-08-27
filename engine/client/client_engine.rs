use deerpunk_engine_common::world::World;
use deerpunk_engine_renderer::gpu::{Renderer, ShaderSources};
use deerpunk_engine_renderer::snapshot::{CameraRenderSnapshot, RenderScene, RenderSnapshot, RenderTarget};

use crate::app::App;
use crate::render_extractor::RenderExtractor;

pub struct ClientEngine {
    app: App,
    renderer: Renderer,
    extractor: Option<RenderExtractor>,
}

impl ClientEngine {
    pub fn new() -> Self {
        let app = App::new("Deerpunk Engine", 1280, 720);

        let instance = wgpu::Instance::default();
        let surface = app.create_wgpu_surface(&instance);

        let mut renderer = Renderer::new(
            load_shader_sources(),
            &instance,
            Some(&surface),
        );

        let (w, h) = app.window_size();
        renderer.set_surface(surface, w, h);

        Self {
            app,
            renderer,
            extractor: None,
        }
    }

    pub fn init_extractor(&mut self, world: &mut World) {
        self.extractor = Some(RenderExtractor::new(world));
    }

    pub fn tick(&mut self, scene: Option<&RenderScene>) -> bool {
        if !self.app.poll_events() {
            return false;
        }

        if let Some(scene) = scene {
            let (w, h) = self.app.window_size();
            let aspect = w as f32 / h as f32;
            let snapshots = scene.build_snapshots(aspect);
            for snap in &snapshots {
                self.render_camera_snapshot(snap);
            }
        } else {
            let snapshot = RenderSnapshot::new([0.1, 0.2, 0.3, 1.0]);
            let _ = self.renderer.render_to_surface(&snapshot);
        }
        true
    }

    fn render_camera_snapshot(&mut self, snap: &CameraRenderSnapshot) {
        match &snap.target {
            RenderTarget::Surface => {
                let mut snapshot = RenderSnapshot::new(snap.clear);
                snapshot.view_proj = snap.view_proj.to_cols_array_2d();
                snapshot.game_time = snap.game_time;
                for md in &snap.visible_meshes {
                    snapshot.mesh_draws.push(md.clone());
                }
                snapshot.sky = snap.sky;
                let _ = self.renderer.render_to_surface(&snapshot);
            }
            _ => {}
        }
    }

    pub fn extract(&self, world: &World) -> RenderScene {
        self.extractor.as_ref()
            .map(|e| e.extract(world))
            .unwrap_or_else(RenderScene::new)
    }

    pub fn window_size(&self) -> (u32, u32) {
        self.app.window_size()
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }
}

fn load_shader_sources() -> ShaderSources {
    let shader_dir = std::env::var("SHADER_DIR").unwrap_or_else(|_| "shaders".to_string());
    let load = |name: &str| -> String {
        let path = format!("{}/{}.wgsl", shader_dir, name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("Failed to load shader {}: {}", path, e);
        })
    };
    ShaderSources {
        sprite: load("sprite"),
        mesh: load("mesh"),
        sky: load("sky"),
        cloud: load("cloud"),
        debug: load("debug"),
        tonemap: load("tonemap"),
        luminance: load("luminance"),
        storm: load("storm"),
    }
}
