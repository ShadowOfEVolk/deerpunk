use deerpunk_engine_common::coord;
use deerpunk_engine_common::frame;
use glam::Vec3;

/// Размер видимой области для Follow-камеры (в метрах).
pub const FOLLOW_VIEW_SIZE: f32 = 5.0;
pub const CAMERA_SMOOTH_SPEED: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    Overview,
    Follow,
    Freecam,
}

/// Состояние клавиш для freecam.
#[derive(Clone, Copy, Debug, Default)]
pub struct FreecamInput {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub dx: f32,
    pub dy: f32,
}

/// Чистая дата-структура камеры без ECS-зависимостей.
/// Методы `update` и `update_camera` остаются в `game` (используют ECS World).
pub struct Camera {
    pub position: Vec3,
    pub mode: CameraMode,
    pub follow_target: Option<u64>,
    /// Пауза над местом смерти перед переключением на другого танка.
    pub death_linger: f32,
    pub linger_x: Option<f32>,
    pub linger_y: Option<f32>,
    pub world_size: f32,
    /// Угол наклона камеры (радианы). 0 = строго вниз, положительный = наклон вперёд.
    pub pitch: f32,
    /// Горизонтальный поворот камеры (радианы). 0 = вдоль +Y.
    pub yaw: f32,
    /// Скорость freecam (м/с).
    pub freecam_speed: f32,
    /// Состояние клавиш WASD + Space/Shift для freecam.
    pub freecam_input: FreecamInput,
}

impl Camera {
    pub fn new(follow_target: Option<u64>, world_size: f32) -> Self {
        Self {
            position: Vec3::new(world_size * 0.5, world_size * 0.5, 10.0),
            mode: CameraMode::Follow,
            follow_target,
            death_linger: 0.0,
            linger_x: None,
            linger_y: None,
            world_size,
            pitch: 0.0,
            yaw: 0.0,
            freecam_speed: 30.0,
            freecam_input: FreecamInput::default(),
        }
    }

    pub fn set_world_size(&mut self, world_size: f32) {
        self.world_size = world_size;
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            CameraMode::Overview => CameraMode::Follow,
            CameraMode::Follow => CameraMode::Freecam,
            CameraMode::Freecam => CameraMode::Overview,
        };
    }

    /// View-projection матрица4x4 (column-major, WGSL layout).
    pub fn view_proj(&self) -> [[f32; 4]; 4] {
        coord::build_view_proj(
            self.position.x,
            self.position.y,
            self.position.z,
            self.yaw,
            self.pitch,
        )
    }

    /// Ортографическая матрица для HUD (экраные пиксели → NDC).
    pub fn hud_view_proj() -> [[f32; 4]; 4] {
        let w = frame::frame_width() as f32;
        let h = frame::frame_height() as f32;
        [
            [2.0 / w, 0.0, 0.0, -1.0],
            [0.0, -2.0 / h, 0.0, 1.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    /// Мировые координаты (x, y, z) → экранные пиксели (px, py).
    /// Если точка за камерой — возвращает None.
    pub fn world_to_screen(&self, wx: f32, wy: f32, wz: f32) -> Option<(f32, f32)> {
        let vp = self.view_proj();
        let w = frame::frame_width() as f32;
        let h = frame::frame_height() as f32;
        // vp is column-major: vp[col][row]
        let cx = vp[0][0] * wx + vp[1][0] * wy + vp[2][0] * wz + vp[3][0];
        let cy = vp[0][1] * wx + vp[1][1] * wy + vp[2][1] * wz + vp[3][1];
        let _cz = vp[0][2] * wx + vp[1][2] * wy + vp[2][2] * wz + vp[3][2];
        let cw = vp[0][3] * wx + vp[1][3] * wy + vp[2][3] * wz + vp[3][3];
        if cw <= 0.0 {
            return None;
        }
        let ndc_x = cx / cw;
        let ndc_y = cy / cw;
        let px = (ndc_x * 0.5 + 0.5) * w;
        let py = (-ndc_y * 0.5 + 0.5) * h;
        Some((px, py))
    }
}

pub fn tank_in_camera_view(x: f32, y: f32, camera: &Camera) -> bool {
    let view_size = camera.position.z / 0.8;
    let ppm = frame::frame_ppm(view_size);
    let half_w = frame::frame_width() as f32 / ppm * 0.5 + frame::TANK_LENGTH;
    let half_h = frame::frame_height() as f32 / ppm * 0.5 + frame::TANK_LENGTH;
    (x - camera.position.x).abs() <= half_w && (y - camera.position.y).abs() <= half_h
}
