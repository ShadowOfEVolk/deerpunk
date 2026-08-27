//! `RenderSnapshot` — CPU-сторона rendersnapshot-паттерна.
//!
//! Каждый кадр игра собирает плоский список draw-примитивов
//! ([`Instance`]) в экранных пикселях (верхний левый угол — (0,0), ось Y вниз).
//! Снапшот отдаётся бэкенду (wgpu), который загружает его в GPU и рисует
//! одним инстансированным draw-вызовом.

use std::sync::Arc;

use tiny_skia::Pixmap;

pub use deerpunk_engine_common::MeshTexture;

/// Один draw-вызов меша в3D.
#[derive(Clone)]
pub struct MeshDraw {
    pub mesh: deerpunk_engine_common::resource::mesh::Mesh,
    pub model_mat: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub texture: MeshTexture,
    /// Направление на солнце (xyz), w = диффузная интенсивность (0 = без света).
    pub sun_dir: [f32; 4],
    /// Ambient-цвет (rgb), добавляется всегда.
    pub ambient: [f32; 4],
}

/// Текстура-источник для примитива (`Instance.tex`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexSel {
    /// Битовый шрифт (белые глифы, tint-ится цветом текста).
    Font = 0,
    /// Атлас спрайтов (символы, объекты и пр.).
    Sprite = 1,
    /// Карта мира (зона-бейк).
    Map = 2,
}

/// Форма примитива (`Instance.shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Текстурированный квад.
    Texture = 0,
    /// Сплошной квад.
    Solid = 1,
    /// Круг (SDF, AA в шейдере).
    Circle = 2,
}

/// Один draw-примитив кадра в экранных координатах.
#[derive(Clone)]
pub struct Instance {
    /// Центр квада в экранных пикселях.
    pub x: f32,
    pub y: f32,
    /// Поворот вокруг центра (рад).
    pub rot: f32,
    /// Полуэкстенты в пикселях.
    pub hw: f32,
    pub hh: f32,
    /// UV-прямоугольник в текстуре-источнике (u0, v0, u1, v1).
    /// Для круга — локальные 0..1 координаты квада (для SDF).
    pub uv: [f32; 4],
    /// Цвет tint (straight RGBA 0..1; alpha — непрозрачность).
    pub color: [f32; 4],
    pub shape: Shape,
    pub tex: TexSel,
    /// Параметры спрайта для `TexSel::Sprite`.
    pub sprite: Option<Arc<Pixmap>>,
}

impl Instance {
    pub fn sprite(
        x: f32,
        y: f32,
        rot: f32,
        hw: f32,
        hh: f32,
        sprite: Arc<Pixmap>,
        opacity: f32,
    ) -> Self {
        Self {
            x,
            y,
            rot,
            hw,
            hh,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, opacity],
            shape: Shape::Texture,
            tex: TexSel::Sprite,
            sprite: Some(sprite),
        }
    }

    pub fn font(x: f32, y: f32, hw: f32, hh: f32, uv: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            x,
            y,
            rot: 0.0,
            hw,
            hh,
            uv,
            color,
            shape: Shape::Texture,
            tex: TexSel::Font,
            sprite: None,
        }
    }

    pub fn map_quad(x: f32, y: f32, hw: f32, hh: f32, uv: [f32; 4]) -> Self {
        Self {
            x,
            y,
            rot: 0.0,
            hw,
            hh,
            uv,
            color: [1.0, 1.0, 1.0, 1.0],
            shape: Shape::Texture,
            tex: TexSel::Map,
            sprite: None,
        }
    }

    pub fn solid(x: f32, y: f32, hw: f32, hh: f32, color: [f32; 4]) -> Self {
        Self {
            x,
            y,
            rot: 0.0,
            hw,
            hh,
            uv: [0.0, 0.0, 1.0, 1.0],
            color,
            shape: Shape::Solid,
            tex: TexSel::Font,
            sprite: None,
        }
    }

    pub fn circle(x: f32, y: f32, radius: f32, color: [f32; 4]) -> Self {
        Self {
            x,
            y,
            rot: 0.0,
            hw: radius,
            hh: radius,
            uv: [0.0, 0.0, 1.0, 1.0],
            color,
            shape: Shape::Circle,
            tex: TexSel::Font,
            sprite: None,
        }
    }

    pub fn with_rot(mut self, r: f32) -> Self {
        self.rot = r;
        self
    }
}

/// Данные для фонового неба (Hosek-Wilkie, рисуется полноэкранным треугольником).
#[derive(Clone, Copy, Debug)]
pub struct SkyDraw {
    /// Направление на солнце (нормализованное) в мировых координатах.
    pub sun_dir: [f32; 3],
    /// Позиция камеры в мире.
    pub camera_pos: [f32; 3],
    /// 30 параметров модели: 27 + 3 масштаба радианса (см. engine::sky).
    pub params: [f32; 30],
    /// Цвет неба для композитинга облаков (zenith или средний sky color).
    pub sky_color: [f32; 3],
}

/// Данные для storm pass (чёрный туман в заражённых тайтлах).
#[derive(Clone)]
pub struct StormDraw {
    /// 64×64 infected bitmap (row-major, 0 = safe, 1 = infected).
    pub infected: Vec<u8>,
    /// World size in meters.
    pub map_size: f32,
    /// Tile size in meters.
    pub tile_meters: f32,
}

/// Полный снапшот кадра.
#[derive(Clone)]
pub struct RenderSnapshot {
    /// Цвет очистки кадра (RGBA 0..1).
    pub clear: [f32; 4],
    /// 3D-примитивы в мировых координатах (отрисовываются с view_proj камеры).
    pub instances: Vec<Instance>,
    /// HUD-примитивы в экранных пикселях (отрисовываются с ортографической проекцией).
    pub hud: Vec<Instance>,
    /// View-projection матрица для3D-примитивов (column-major).
    pub view_proj: [[f32; 4]; 4],
    /// Inverse view-projection (computed from view_proj).
    pub inv_view_proj: [[f32; 4]; 4],
    /// Меши для отрисовки.
    pub mesh_draws: Vec<MeshDraw>,
    /// Фоновое небо (если задано — рисуется перед 3D-сценой).
    pub sky: Option<SkyDraw>,
    /// Debug-примитивы (линии, сферы, текст) — рисуются поверх 3D-сцены.
    pub debug_draw: Vec<crate::debug_draw::DebugPrimitive>,
    /// Storm fog overlay (black fog in infected tiles).
    pub storm: Option<StormDraw>,
    /// Game time in seconds (for animated effects like clouds).
    pub game_time: f32,
}

impl RenderSnapshot {
    pub fn new(clear: [f32; 4]) -> Self {
        Self {
            clear,
            instances: Vec::new(),
            hud: Vec::new(),
            mesh_draws: Vec::new(),
            sky: None,
            debug_draw: Vec::new(),
            storm: None,
            game_time: 0.0,
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            inv_view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn push(&mut self, inst: Instance) {
        self.instances.push(inst);
    }

    pub fn push_hud(&mut self, inst: Instance) {
        self.hud.push(inst);
    }

    pub fn push_mesh(&mut self, mesh: deerpunk_engine_common::resource::mesh::Mesh, model_mat: [[f32; 4]; 4], color: [f32; 4], texture: MeshTexture) {
        self.mesh_draws.push(MeshDraw {
            mesh,
            model_mat,
            color,
            texture,
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [1.0, 1.0, 1.0, 0.0],
        });
    }

    pub fn push_mesh_lit(&mut self, mesh: deerpunk_engine_common::resource::mesh::Mesh, model_mat: [[f32; 4]; 4], color: [f32; 4], texture: MeshTexture, sun_dir: [f32; 4], ambient: [f32; 4]) {
        self.mesh_draws.push(MeshDraw {
            mesh,
            model_mat,
            color,
            texture,
            sun_dir,
            ambient,
        });
    }
}

/// Хелпер: [u8;3] → [f32;4] RGBA.
pub fn rgba_u8(rgb: [u8; 3], a: u8) -> [f32; 4] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

/// Хелпер: [u8;4] → [f32;4] RGBA.
pub fn rgba_u8_4(rgba: [u8; 4]) -> [f32; 4] {
    [
        rgba[0] as f32 / 255.0,
        rgba[1] as f32 / 255.0,
        rgba[2] as f32 / 255.0,
        rgba[3] as f32 / 255.0,
    ]
}

// ── Scene building ───────────────────────────────────────────────────

/// Куда камера рендерит (аналог Unity Camera.target).
#[derive(Clone, Debug)]
pub enum RenderTarget {
    Surface,
    Texture {
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    },
    None,
}

/// Определение камеры в сцене.
#[derive(Clone)]
pub struct CameraDefinition {
    pub position: [f32; 3],
    pub rotation: glam::Quat,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub target: RenderTarget,
    pub dirty: bool,
}

impl Default for CameraDefinition {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 5.0],
            rotation: glam::Quat::IDENTITY,
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 1024.0,
            target: RenderTarget::Surface,
            dirty: true,
        }
    }
}

impl CameraDefinition {
    pub fn view_matrix(&self) -> glam::Mat4 {
        let eye = glam::Vec3::from(self.position);
        let forward = self.rotation * glam::Vec3::NEG_Z;
        let up = self.rotation * glam::Vec3::Y;
        glam::Mat4::look_to_rh(eye, forward, up)
    }

    pub fn projection_matrix(&self, aspect: f32) -> glam::Mat4 {
        glam::Mat4::perspective_rh(self.fov_y, aspect, self.near, self.far)
    }
}

/// Render graph привязанный к камере. Перестраивается при dirty flag.
pub struct CameraRenderGraph {
    pub graph: crate::render_graph::RenderGraph,
    pub dirty: bool,
}

impl CameraRenderGraph {
    pub fn new(graph: crate::render_graph::RenderGraph) -> Self {
        Self { graph, dirty: true }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

/// Геометрия сцены — всё что можно рендерить.
#[derive(Clone)]
pub struct SceneGeometry {
    pub mesh_draws: Vec<MeshDraw>,
    pub sky: Option<SkyDraw>,
}

impl Default for SceneGeometry {
    fn default() -> Self {
        Self {
            mesh_draws: Vec::new(),
            sky: None,
        }
    }
}

/// Собранный снапшот одной камеры: видимая геометрия + матрицы.
#[derive(Clone)]
pub struct CameraRenderSnapshot {
    pub view: glam::Mat4,
    pub projection: glam::Mat4,
    pub view_proj: glam::Mat4,
    pub visible_meshes: Vec<MeshDraw>,
    pub sky: Option<SkyDraw>,
    pub clear: [f32; 4],
    pub game_time: f32,
    pub target: RenderTarget,
}

/// Полная сцена рендеринга — камеры + геометрия.
pub struct RenderScene {
    pub cameras: Vec<CameraDefinition>,
    pub geometry: SceneGeometry,
    pub clear: [f32; 4],
    pub game_time: f32,
}

impl RenderScene {
    pub fn new() -> Self {
        Self {
            cameras: Vec::new(),
            geometry: SceneGeometry::default(),
            clear: [0.1, 0.1, 0.15, 1.0],
            game_time: 0.0,
        }
    }

    pub fn push_camera(&mut self, cam: CameraDefinition) {
        self.cameras.push(cam);
    }

    pub fn set_geometry(&mut self, geo: SceneGeometry) {
        self.geometry = geo;
    }

    /// Для каждой камеры строит CameraRenderSnapshot с frustum-culled
    /// геометрией и view/proj матрицами.
    pub fn build_snapshots(&self, aspect: f32) -> Vec<CameraRenderSnapshot> {
        self.cameras
            .iter()
            .map(|cam| {
                let view = cam.view_matrix();
                let projection = cam.projection_matrix(aspect);
                let view_proj = projection * view;

                let visible_meshes = self.geometry.mesh_draws.clone();

                CameraRenderSnapshot {
                    view,
                    projection,
                    view_proj,
                    visible_meshes,
                    sky: self.geometry.sky,
                    clear: self.clear,
                    game_time: self.game_time,
                    target: cam.target.clone(),
                }
            })
            .collect()
    }
}
