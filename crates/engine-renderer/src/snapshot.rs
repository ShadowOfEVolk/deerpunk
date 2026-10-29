//! `RenderSnapshot` — CPU-сторона rendersnapshot-паттерна.
//!
//! Каждый кадр игра собирает плоский список draw-примитивов
//! ([`Instance`]) в экранных пикселях (верхний левый угол — (0,0), ось Y вниз).
//! Снапшот отдаётся бэкенду (wgpu), который загружает его в GPU и рисует
//! одним инстансированным draw-вызовом.

use std::sync::Arc;

use tiny_skia::Pixmap;

use crate::aabb::Aabb;
use crate::cull::{
    cull_mesh_indices, cull_skinned_indices, CullCamera, CullScratch, CullStats, DEFAULT_CELL_SIZE,
};

pub use deerpunk_engine_common::material::ShaderId;
pub use deerpunk_engine_common::MeshTexture;

/// Один draw-вызов меша в3D.
#[derive(Clone)]
pub struct MeshDraw {
    pub mesh: Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    pub model_mat: [[f32; 4]; 4],
    /// The mesh is already in world space and can be submitted through the
    /// renderer's material/buffer multi-draw path.
    pub batchable: bool,
    pub material_id: u32,
    pub texture: MeshTexture,
    pub normal: MeshTexture,
    pub orm: MeshTexture,
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
    /// Shader from [`ShaderRegistry`] (`0` = invalid — draw skipped).
    pub shader_id: ShaderId,
    /// GPU lightmap pool id when the material shader binds `texture_2d_array` (`0` = white).
    pub lightmap_gpu: u32,
    /// Submesh index range into `mesh.indices`. `index_count == 0` → full mesh.
    pub first_index: u32,
    pub index_count: u32,
    /// World-space bounds for frustum culling (set during scene extract).
    pub world_bounds: Aabb,
    /// ECS entity id for persistent spatial cull (`0` = fallback to draw index).
    pub entity_id: u64,
    /// Copied to `MeshUniform.material_data.y` (e.g. mining crack stage).
    pub uniform_aux: u32,
}

/// Skinned mesh draw — [`bone_buffer`] indexes [`RenderSnapshot::bone_palettes`].
#[derive(Clone)]
pub struct SkinnedMeshDraw {
    pub mesh: Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    pub model_mat: [[f32; 4]; 4],
    pub material_id: u32,
    pub texture: MeshTexture,
    pub normal: MeshTexture,
    pub orm: MeshTexture,
    pub shader_id: ShaderId,
    pub bone_buffer: u32,
    pub world_bounds: Aabb,
    pub entity_id: u64,
}

/// Column-major `Mat4` bone palette for one [`SkinnedMeshDraw::bone_buffer`].
#[derive(Clone)]
pub struct BonePaletteUpload {
    pub handle: u32,
    pub matrices: Vec<[[f32; 4]; 4]>,
}

/// Один draw-вызов террейна со сплат-маппингом.
#[derive(Clone)]
pub struct TerrainSplatDraw {
    pub mesh: Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    pub model_mat: [[f32; 4]; 4],
    pub layers: [MeshTexture; 4],
    pub splat_map: MeshTexture,
    pub sun_dir: [f32; 4],
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

/// Данные для фонового неба (gamelib Hosek + fullscreen triangle).
#[derive(Clone, Copy, Debug)]
pub struct SkyDraw {
    /// `scene_sun_dir()` at extract — copied verbatim into GPU `SkyUniform.sun_dir`.
    pub scene_sun: [f32; 4],
    /// Направление на солнце (нормализованное) в мировых координатах.
    pub sun_dir: [f32; 3],
    /// Позиция камеры в мире.
    pub camera_pos: [f32; 3],
    /// 27 коэффициентов HW + 3 масштаба радианса + exposure + day_fade
    /// (gamelib `SkyEnvironment`, e.g. ohmyblock `shared-sky`).
    pub params: [f32; 32],
    /// Линейный цвет горизонта (дымка) — для облаков и тумана.
    pub sky_color: [f32; 3],
    /// Линейный зенит (купол), с CPU Hosek.
    pub zenith: [f32; 3],
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
    /// World-space camera position.
    pub camera_pos: [f32; 3],
    /// Меши для отрисовки.
    pub mesh_draws: Vec<MeshDraw>,
    pub skinned_mesh_draws: Vec<SkinnedMeshDraw>,
    pub bone_palettes: Vec<BonePaletteUpload>,
    /// Террейн со сплат-маппингом.
    pub terrain_splat_draws: Vec<TerrainSplatDraw>,
    /// Forward point lights (capped by renderer).
    pub point_lights: Vec<PointLightDraw>,
    /// Фоновое небо (если задано — рисуется перед 3D-сценой).
    pub sky: Option<SkyDraw>,
    /// Debug-примитивы (линии, сферы, текст) — рисуются поверх 3D-сцены.
    pub debug_draw: Vec<crate::debug_draw::DebugPrimitive>,
    /// Storm fog overlay (black fog in infected tiles).
    pub storm: Option<StormDraw>,
    /// Game time in seconds (for animated effects like clouds).
    pub game_time: f32,
    /// Frustum cull stats for the active camera (when enabled).
    pub cull_stats: CullStats,
    pub frustum_culled: bool,
    /// BSP / map static lights visible this frame (gamelib may set).
    pub visible_world_lights: u32,
    /// PVS occluder volumes tested (reserved).
    pub visible_occluders: u32,
}

impl RenderSnapshot {
    pub fn new(clear: [f32; 4]) -> Self {
        Self {
            clear,
            instances: Vec::new(),
            hud: Vec::new(),
            mesh_draws: Vec::new(),
            skinned_mesh_draws: Vec::new(),
            bone_palettes: Vec::new(),
            terrain_splat_draws: Vec::new(),
            point_lights: Vec::new(),
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
            camera_pos: [0.0, 0.0, 0.0],
            cull_stats: CullStats::default(),
            frustum_culled: false,
            visible_world_lights: 0,
            visible_occluders: 0,
        }
    }

    pub fn push(&mut self, inst: Instance) {
        self.instances.push(inst);
    }

    pub fn push_hud(&mut self, inst: Instance) {
        self.hud.push(inst);
    }

    pub fn push_mesh(
        &mut self,
        mesh: deerpunk_engine_common::resource::mesh::Mesh,
        model_mat: [[f32; 4]; 4],
        material_id: u32,
        texture: MeshTexture,
        shader_id: ShaderId,
        lightmap_gpu: u32,
    ) {
        let model = glam::Mat4::from_cols_array_2d(&model_mat);
        let bounds = crate::cull::world_bounds(model, &mesh);
        self.mesh_draws.push(MeshDraw {
            mesh: Arc::new(mesh),
            model_mat,
            batchable: false,
            material_id,
            texture,
            normal: MeshTexture::WHITE,
            orm: MeshTexture::WHITE,
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [1.0, 1.0, 1.0, 0.0],
            shader_id,
            lightmap_gpu,
            first_index: 0,
            index_count: 0,
            world_bounds: bounds,
            entity_id: 0,
            uniform_aux: 0,
        });
    }

    pub fn push_mesh_lit(
        &mut self,
        mesh: deerpunk_engine_common::resource::mesh::Mesh,
        model_mat: [[f32; 4]; 4],
        material_id: u32,
        texture: MeshTexture,
        shader_id: ShaderId,
        lightmap_gpu: u32,
        sun_dir: [f32; 4],
        ambient: [f32; 4],
    ) {
        let model = glam::Mat4::from_cols_array_2d(&model_mat);
        let bounds = crate::cull::world_bounds(model, &mesh);
        self.mesh_draws.push(MeshDraw {
            mesh: Arc::new(mesh),
            model_mat,
            batchable: false,
            material_id,
            texture,
            normal: MeshTexture::WHITE,
            orm: MeshTexture::WHITE,
            sun_dir,
            ambient,
            shader_id,
            lightmap_gpu,
            first_index: 0,
            index_count: 0,
            world_bounds: bounds,
            entity_id: 0,
            uniform_aux: 0,
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
        let forward = self.rotation * glam::Vec3::Y;
        let up = self.rotation * glam::Vec3::Z;
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

/// One forward point light for the frame.
#[derive(Clone, Copy, Debug)]
pub struct PointLightDraw {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
}

/// Геометрия сцены — всё что можно рендерить.
#[derive(Clone)]
pub struct SceneGeometry {
    pub mesh_draws: Vec<MeshDraw>,
    pub skinned_mesh_draws: Vec<SkinnedMeshDraw>,
    pub bone_palettes: Vec<BonePaletteUpload>,
    pub terrain_splat_draws: Vec<TerrainSplatDraw>,
    pub sky: Option<SkyDraw>,
    pub point_lights: Vec<PointLightDraw>,
    /// Hash of mesh + skinned draw keys/bounds; bump via [`Self::refresh_cull_serial`].
    pub cull_serial: u64,
}

impl SceneGeometry {
    /// Recompute [`Self::cull_serial`] once per extract (O(n); cull uses O(1) compare).
    pub fn refresh_cull_serial(&mut self) {
        self.cull_serial = crate::cull::scene_geometry_cull_serial(self);
    }
}

impl Default for SceneGeometry {
    fn default() -> Self {
        Self {
            mesh_draws: Vec::new(),
            skinned_mesh_draws: Vec::new(),
            bone_palettes: Vec::new(),
            terrain_splat_draws: Vec::new(),
            sky: None,
            point_lights: Vec::new(),
            cull_serial: 0,
        }
    }
}

/// Собранный снапшот одной камеры: матрицы + индексы в [`RenderScene::geometry`].
#[derive(Clone)]
pub struct CameraRenderSnapshot {
    pub view: glam::Mat4,
    pub projection: glam::Mat4,
    pub view_proj: glam::Mat4,
    pub inv_view_proj: glam::Mat4,
    /// Indices into `geometry.mesh_draws` when [`frustum_culled`] is true.
    pub visible_mesh: Vec<u32>,
    /// Indices into `geometry.skinned_mesh_draws` when [`frustum_culled`] is true.
    pub visible_skinned: Vec<u32>,
    /// If false, use full `geometry.*` draw lists (culling off).
    pub frustum_culled: bool,
    pub bone_palettes: Vec<BonePaletteUpload>,
    pub point_lights: Vec<PointLightDraw>,
    pub sky: Option<SkyDraw>,
    pub clear: [f32; 4],
    pub game_time: f32,
    pub target: RenderTarget,
    pub cull_stats: CullStats,
}

/// Полная сцена рендеринга — камеры + геометрия.
/// Frustum-culling toggles for [`RenderScene::build_snapshots`].
#[derive(Clone, Copy, Debug)]
pub struct CullOptions {
    pub enabled: bool,
    pub cell_size: f32,
}

impl Default for CullOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            cell_size: DEFAULT_CELL_SIZE,
        }
    }
}

pub struct RenderScene {
    pub cameras: Vec<CameraDefinition>,
    pub geometry: SceneGeometry,
    pub clear: [f32; 4],
    pub game_time: f32,
    pub cull: CullOptions,
}

impl RenderScene {
    pub fn new() -> Self {
        Self {
            cameras: Vec::new(),
            geometry: SceneGeometry::default(),
            clear: [0.1, 0.1, 0.15, 1.0],
            game_time: 0.0,
            cull: CullOptions::default(),
        }
    }

    pub fn push_camera(&mut self, cam: CameraDefinition) {
        self.cameras.push(cam);
    }

    pub fn set_geometry(&mut self, geo: SceneGeometry) {
        self.geometry = geo;
    }

    /// Для каждой камеры строит CameraRenderSnapshot (индексы видимых draw'ов).
    pub fn build_snapshots(
        &self,
        aspect: f32,
        scratch: &mut CullScratch,
    ) -> Vec<CameraRenderSnapshot> {
        scratch.reset_build_timing();
        self.cameras
            .iter()
            .map(|cam| {
                let view = cam.view_matrix();
                let projection = cam.projection_matrix(aspect);
                let view_proj = projection * view;
                let cull_cam = CullCamera {
                    view_proj,
                    view,
                    near: cam.near,
                    far: cam.far,
                    fov_y: cam.fov_y,
                    aspect,
                };

                let frustum_start = std::time::Instant::now();
                let cull_serial = self.geometry.cull_serial;
                let (frustum_culled, cull_stats, visible_mesh, visible_skinned) =
                    if self.cull.enabled {
                        let (mt, mv) = cull_mesh_indices(
                            &self.geometry.mesh_draws,
                            &cull_cam,
                            cull_serial,
                            self.cull.cell_size,
                            scratch,
                        );
                        let visible_mesh = std::mem::take(&mut scratch.visible_mesh);
                        let (st, sv) = cull_skinned_indices(
                            &self.geometry.skinned_mesh_draws,
                            &cull_cam,
                            cull_serial,
                            self.cull.cell_size,
                            scratch,
                        );
                        let visible_skinned = std::mem::take(&mut scratch.visible_skinned);
                        scratch.last_frustum_ms += frustum_start.elapsed().as_secs_f32() * 1000.0;
                        (
                            true,
                            CullStats {
                                mesh_total: mt,
                                mesh_visible: mv,
                                skinned_total: st,
                                skinned_visible: sv,
                            },
                            visible_mesh,
                            visible_skinned,
                        )
                    } else {
                        let n = self.geometry.mesh_draws.len() as u32;
                        let s = self.geometry.skinned_mesh_draws.len() as u32;
                        scratch.last_frustum_ms += frustum_start.elapsed().as_secs_f32() * 1000.0;
                        (
                            false,
                            CullStats {
                                mesh_total: n,
                                mesh_visible: n,
                                skinned_total: s,
                                skinned_visible: s,
                            },
                            Vec::new(),
                            Vec::new(),
                        )
                    };

                let assemble_start = std::time::Instant::now();
                let inv_view_proj = view_proj.inverse();
                let snap = CameraRenderSnapshot {
                    view,
                    projection,
                    view_proj,
                    inv_view_proj,
                    visible_mesh,
                    visible_skinned,
                    frustum_culled,
                    bone_palettes: self.geometry.bone_palettes.clone(),
                    point_lights: self.geometry.point_lights.clone(),
                    sky: self.geometry.sky,
                    clear: self.clear,
                    game_time: self.game_time,
                    target: cam.target.clone(),
                    cull_stats,
                };
                scratch.last_assemble_ms += assemble_start.elapsed().as_secs_f32() * 1000.0;
                snap
            })
            .collect()
    }

    /// Copy visible draws into GPU snapshot buffers (single gather pass per frame).
    pub fn gather_visible_draws(
        &self,
        cam: &CameraRenderSnapshot,
        mesh_out: &mut Vec<MeshDraw>,
        skinned_out: &mut Vec<SkinnedMeshDraw>,
    ) {
        mesh_out.clear();
        skinned_out.clear();
        if cam.frustum_culled {
            mesh_out.reserve(cam.visible_mesh.len());
            for &i in &cam.visible_mesh {
                mesh_out.push(self.geometry.mesh_draws[i as usize].clone());
            }
            skinned_out.reserve(cam.visible_skinned.len());
            for &i in &cam.visible_skinned {
                skinned_out.push(self.geometry.skinned_mesh_draws[i as usize].clone());
            }
        } else {
            mesh_out.clone_from(&self.geometry.mesh_draws);
            skinned_out.clone_from(&self.geometry.skinned_mesh_draws);
        }
    }
}
