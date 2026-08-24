use deerpunk_engine_common::cvar::cvar;
use deerpunk_engine_common::cmd::{cmd, exec_plus_args};
use deerpunk_engine_host::Host;
use deerpunk_engine_host::ServerEngine;
use std::sync::Arc;
use std::time::Instant;

use deerpunk_engine_renderer::gpu::{Renderer, ShaderSources};
use deerpunk_engine_renderer::snapshot::{RenderSnapshot, MeshDraw};

use deerpunk_engine_common::Transform;
use deerpunk_engine_common::MeshRenderer;
use deerpunk_engine_common::MeshTexture;
use deerpunk_engine_common::Terrain;
use deerpunk_engine_common::TerrainRenderer;

use glam::{Mat4, Vec3};

use winit::application::ApplicationHandler;
use winit::event::{WindowEvent, DeviceEvent, DeviceId};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

type ServerInitFn = unsafe extern "C" fn(*const deerpunk_engine_host::vtable::EngineApiVtable);
type ServerFrameFn = unsafe extern "C" fn(f32);
type ServerShutdownFn = unsafe extern "C" fn();

struct ServerDll {
    _lib: libloading::Library,
    init_fn: ServerInitFn,
    frame_fn: ServerFrameFn,
    shutdown_fn: ServerShutdownFn,
}

impl ServerDll {
    fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let lib = unsafe { libloading::Library::new(path) }?;
        let init_fn = unsafe { *lib.get::<ServerInitFn>(b"server_init")? };
        let frame_fn = unsafe { *lib.get::<ServerFrameFn>(b"server_frame")? };
        let shutdown_fn = unsafe { *lib.get::<ServerShutdownFn>(b"server_shutdown")? };
        Ok(Self { _lib: lib, init_fn, frame_fn, shutdown_fn })
    }
    unsafe fn init(&self, vtable_ptr: *const deerpunk_engine_host::vtable::EngineApiVtable) { (self.init_fn)(vtable_ptr); }
    unsafe fn frame(&self, dt: f32) { (self.frame_fn)(dt); }
    unsafe fn shutdown(&self) { (self.shutdown_fn)(); }
}

pub const TEX_GROUND: MeshTexture = MeshTexture::slot(2);

fn load_ground_texture() -> (Vec<u8>, u32, u32) {
    let grass_path = std::env::var("TEXTURE_DIR")
        .map(|d| format!("{}/tiles/grass.png", d))
        .unwrap_or_else(|_| "../tanks/assets/tiles/grass.png".to_string());
    match image::open(&grass_path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            println!("[engine] loaded grass texture {}x{} from {}", w, h, grass_path);
            (rgba.into_raw(), w, h)
        }
        Err(e) => {
            eprintln!("[engine] failed to load grass texture: {}, using solid green", e);
            let solid = [[60u8, 140, 50, 255]].repeat(256 * 256);
            (solid.into_iter().flatten().collect::<Vec<u8>>(), 256, 256)
        }
    }
}

fn load_shader_sources() -> ShaderSources {
    let shader_dir = std::env::var("SHADER_DIR")
        .unwrap_or_else(|_| "shaders".to_string());
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

fn build_terrain_mesh(heights: &[f32], rows: usize, cols: usize, cell_size: f32, height_scale: f32) -> deerpunk_engine_common::resource::mesh::Mesh {
    let mut positions = Vec::with_capacity(rows * cols);
    let mut normals = Vec::with_capacity(rows * cols);
    let mut uvs = Vec::with_capacity(rows * cols);
    let mut indices = Vec::with_capacity((rows - 1) * (cols - 1) * 6);

    for z in 0..rows {
        for x in 0..cols {
            let px = x as f32 * cell_size;
            let py = z as f32 * cell_size;
            let h = heights[z * cols + x] * height_scale;
            positions.push([px, py, h]);
            uvs.push([x as f32 / (cols - 1) as f32, z as f32 / (rows - 1) as f32]);
        }
    }

    for z in 0..rows {
        for x in 0..cols {
            let h_l = heights[z * cols + x.saturating_sub(1)] * height_scale;
            let h_r = heights[z * cols + (x + 1).min(cols - 1)] * height_scale;
            let h_d = heights[z.saturating_sub(1) * cols + x] * height_scale;
            let h_u = heights[(z + 1).min(rows - 1) * cols + x] * height_scale;
            let dx = h_r - h_l;
            let dy = h_u - h_d;
            let len = (dx * dx + dy * dy + 4.0 * cell_size * cell_size).sqrt();
            normals.push([-dx / len, -dy / len, 2.0 * cell_size / len]);
        }
    }

    for z in 0..(rows - 1) {
        for x in 0..(cols - 1) {
            let tl = (z * cols + x) as u32;
            let tr = (z * cols + x + 1) as u32;
            let bl = ((z + 1) * cols + x) as u32;
            let br = ((z + 1) * cols + x + 1) as u32;
            indices.extend_from_slice(&[tl, tr, bl, tr, br, bl]);
        }
    }

    deerpunk_engine_common::resource::mesh::Mesh { positions, normals, uvs, indices }
}

fn build_snapshot(host: &Host, window_size: (u32, u32)) -> RenderSnapshot {
    let mut snapshot = RenderSnapshot::new([0.1, 0.15, 0.3, 1.0]);
    let se = &host.server_engine;

    let aspect = window_size.0 as f32 / window_size.1 as f32;

    let transform_id = match se.resolve_component("Transform") {
        Some(id) => id,
        None => return snapshot,
    };
    let mesh_renderer_id = match se.resolve_component("MeshRenderer") {
        Some(id) => id,
        None => return snapshot,
    };
    let camera_id = se.resolve_component("CameraComponent");

    let (cam_pos, cam_fov, cam_near, cam_far, cam_rotation) = if let Some(cid) = camera_id {
        let cam_entities = se.query(&[transform_id, cid], &[]);
        if let Some(&eid) = cam_entities.first() {
            let t: Transform = match se.get_raw(eid, transform_id) {
                Some(ptr) => unsafe { std::ptr::read_unaligned(ptr as *const Transform) },
                None => Transform::new(0.0, -8.0, 6.0),
            };
            let cam: deerpunk_engine_common::CameraComponent = match se.get_raw(eid, cid) {
                Some(ptr) => unsafe { std::ptr::read_unaligned(ptr as *const deerpunk_engine_common::CameraComponent) },
                None => deerpunk_engine_common::CameraComponent::default(),
            };
            (t.position, cam.fov_y, cam.near, cam.far, Some(t.forward()))
        } else {
            (Vec3::new(0.0, -8.0, 6.0), 60.0_f32.to_radians(), 0.1, 1024.0, None)
        }
    } else {
        (Vec3::new(0.0, -8.0, 6.0), 60.0_f32.to_radians(), 0.1, 1024.0, None)
    };

    let proj = Mat4::perspective_rh(cam_fov, aspect, cam_near, cam_far);
    let view = match cam_rotation {
        Some(forward) => Mat4::look_at_rh(cam_pos, cam_pos + forward, Vec3::Z),
        None => Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Z),
    };
    let view_proj = proj * view;
    let inv_view_proj = view_proj.inverse();

    snapshot.view_proj = view_proj.to_cols_array_2d();
    snapshot.inv_view_proj = inv_view_proj.to_cols_array_2d();

    let transform_id = match se.resolve_component("Transform") {
        Some(id) => id,
        None => return snapshot,
    };
    let mesh_renderer_id = match se.resolve_component("MeshRenderer") {
        Some(id) => id,
        None => return snapshot,
    };

    let entities = se.query(&[transform_id, mesh_renderer_id], &[]);

    for &entity_id in &entities {
        let t: Transform = match se.get_raw(entity_id, transform_id) {
            Some(ptr) => unsafe { std::ptr::read_unaligned(ptr as *const Transform) },
            None => continue,
        };

        if !se.has(entity_id, mesh_renderer_id) {
            continue;
        }

        let (color, texture, sun_dir, ambient) =
            match se.get_raw(entity_id, mesh_renderer_id) {
                Some(ptr) => {
                    unsafe {
                        let mr = &*(ptr as *const deerpunk_engine_common::MeshRenderer);
                        (mr.color, mr.texture, mr.sun_dir, mr.ambient)
                    }
                }
                None => continue,
            };

        snapshot.mesh_draws.push(MeshDraw {
            mesh: deerpunk_engine_common::resource::mesh::Mesh::cube(),
            model_mat: t.model_mat(),
            color,
            texture,
            sun_dir,
            ambient,
        });
    }

    // Terrain rendering
    if let (Some(terrain_id), Some(tr_id)) = (
        se.resolve_component("Terrain"),
        se.resolve_component("TerrainRenderer"),
    ) {
        let terrain_entities = se.query(&[transform_id, terrain_id, tr_id], &[]);
        for &entity_id in &terrain_entities {
            let t: Transform = match se.get_raw(entity_id, transform_id) {
                Some(ptr) => unsafe { std::ptr::read_unaligned(ptr as *const Transform) },
                None => continue,
            };

            let (mesh, color, texture, sun_dir, ambient) = unsafe {
                let terrain = match se.get_raw(entity_id, terrain_id) {
                    Some(ptr) => &*(ptr as *const deerpunk_engine_common::Terrain),
                    None => continue,
                };

                if terrain.heights.is_empty() || terrain.rows == 0 || terrain.cols == 0
                    || terrain.rows > 4096 || terrain.cols > 4096
                {
                    continue;
                }

                let mesh = build_terrain_mesh(&terrain.heights, terrain.rows, terrain.cols, terrain.cell_size, terrain.height_scale);

                let tr = match se.get_raw(entity_id, tr_id) {
                    Some(ptr) => &*(ptr as *const deerpunk_engine_common::TerrainRenderer),
                    None => continue,
                };

                (mesh, tr.color, tr.texture, tr.sun_dir, tr.ambient)
            };

            snapshot.mesh_draws.push(MeshDraw {
                mesh,
                model_mat: t.model_mat(),
                color,
                texture,
                sun_dir,
                ambient,
            });
        }
    }

    snapshot
}

struct EngineApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    host: Option<Host>,
    server: Option<ServerDll>,
    last_frame_time: Instant,
    game_time: f32,
}

impl EngineApp {
    fn new() -> Self {
        Self { window: None, renderer: None, host: None, server: None, last_frame_time: Instant::now(), game_time: 0.0 }
    }

    fn init(&mut self) {
        cvar().initialize();
        cmd().initialize();
        exec_plus_args(&[]);
        let mut host = Host::new();
        let gamedir = cvar().get_string("gamedir").unwrap_or_else(|| "examples/deathmatch/gamedir".to_string());
        host.server_engine.initialize(gamedir).unwrap_or_else(|e| eprintln!("init error: {}", e));
        let handle = host.build_vtable();
        let vtable_ptr = handle.vtable_ptr();
        let dll_path = std::env::var("SERVER_DLL").unwrap_or_else(|_| "dlls/server.dll".to_string());
        let server = match ServerDll::load(&dll_path) {
            Ok(s) => { unsafe { s.init(vtable_ptr) }; Some(s) }
            Err(e) => { eprintln!("DLL load failed: {}. Headless.", e); None }
        };
        self.host = Some(host);
        self.server = server;
        if let Some(ref mut h) = self.host {
            let ctx = &mut h.server_engine as *mut ServerEngine as *mut std::ffi::c_void;
            deerpunk_engine_host::set_engine_ctx(ctx);
        }
    }
}

impl ApplicationHandler for EngineApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let attrs = Window::default_attributes()
            .with_title("Deerpunk Engine")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attrs).expect("Failed to create window"));

        let mut renderer = Renderer::new(load_shader_sources());
        let size = window.inner_size();
        let surface = renderer.create_surface(Arc::clone(&window))
            .expect("Failed to create surface");
        renderer.set_surface(surface, size.width, size.height);

        let (ground_pixels, gw, gh) = load_ground_texture();
        renderer.upload_mesh_texture(TEX_GROUND, &ground_pixels, gw, gh);

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.init();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                std::process::exit(0);
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.resize_surface(size.width, size.height);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let winit::event::ElementState::Pressed | winit::event::ElementState::Released = event.state {
                    let down = matches!(event.state, winit::event::ElementState::Pressed);
                    use winit::keyboard::KeyCode;
                    let vk = match event.physical_key {
                        winit::keyboard::PhysicalKey::Code(k) => match k {
                            KeyCode::KeyW => Some(0x57),
                            KeyCode::KeyA => Some(0x41),
                            KeyCode::KeyS => Some(0x53),
                            KeyCode::KeyD => Some(0x44),
                            KeyCode::Space => Some(0x20),
                            KeyCode::ShiftLeft => Some(0xA0),
                            KeyCode::Escape => Some(0x1B),
                            KeyCode::KeyE => Some(0x45),
                            KeyCode::KeyQ => Some(0x51),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(vk) = vk {
                        deerpunk_engine_common::input::set_key(vk, down);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame_time).as_secs_f32().min(0.1);
                self.last_frame_time = now;
                self.game_time += dt;

                if let Some(ref mut renderer) = self.renderer {
                    let size = self.window.as_ref().map_or((1280, 720), |w| {
                        let s = w.inner_size();
                        (s.width, s.height)
                    });
                    renderer.imgui_begin_frame(size.0 as f32, size.1 as f32);
                }
                if let Some(ref server) = self.server {
                    unsafe { server.frame(dt) };
                }
                if let Some(ref mut host) = self.host {
                    let _ = host.server_engine.frame(dt);
                }

                if let (Some(renderer), Some(host), Some(window)) =
                    (self.renderer.as_mut(), self.host.as_ref(), self.window.as_ref())
                {
                    let size = window.inner_size();
                    let mut snapshot = build_snapshot(host, (size.width, size.height));
                    snapshot.game_time = self.game_time;
                    let _ = renderer.render_to_surface(&snapshot);
                }

                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
        match event {
            DeviceEvent::MouseMotion { delta } => {
                deerpunk_engine_common::input::set_mouse_delta(delta.0 as f32, delta.1 as f32);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = EngineApp::new();
    event_loop.run_app(&mut app).expect("Event loop error");
}
