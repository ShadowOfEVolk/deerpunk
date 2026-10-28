use deerpunk_engine_common::components::TransformComponent;
use deerpunk_engine_common::coord;
use deerpunk_engine_common::frame;
use deerpunk_engine_common::input::Input;
use glam::{Quat, Vec3};

pub const FOLLOW_VIEW_SIZE: f32 = 5.0;
pub const CAMERA_SMOOTH_SPEED: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    Overview,
    Follow,
    Freecam,
}

pub struct FreecamController {
    pub speed: f32,
    pub sensitivity: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for FreecamController {
    fn default() -> Self {
        Self {
            speed: 30.0,
            sensitivity: 0.003,
            yaw: 0.0,
            pitch: -0.3,
        }
    }
}

impl FreecamController {
    pub fn update(&mut self, input: &Input, transform: &mut TransformComponent, dt: f32) {
        self.yaw += input.mouse_dx() * self.sensitivity;
        self.pitch = (self.pitch - input.mouse_dy() * self.sensitivity)
            .clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);

        let ep = self.pitch + std::f32::consts::FRAC_PI_2;
        let forward = Vec3::new(
            -self.yaw.sin() * ep.cos(),
            self.yaw.cos() * ep.cos(),
            ep.sin(),
        );
        let right = Vec3::new(self.yaw.cos(), self.yaw.sin(), 0.0);
        let up = Vec3::Z;

        let mut move_dir = Vec3::ZERO;
        if input.is_down(0x57) { move_dir += forward; }  // W
        if input.is_down(0x53) { move_dir -= forward; }  // S
        if input.is_down(0x44) { move_dir += right; }    // D
        if input.is_down(0x41) { move_dir -= right; }    // A
        if input.is_down(0x20) { move_dir += up; }       // Space
        if input.is_down(0xA0) { move_dir -= up; }       // LShift

        if move_dir.length_squared() > 0.0 {
            move_dir = move_dir.normalize();
        }

        let speed = if input.is_down(0xA2) { self.speed * 3.0 } else { self.speed }; // LCtrl = fast
        transform.position += move_dir * speed * dt;
        transform.rotation = Quat::from_rotation_z(-self.yaw) * Quat::from_rotation_x(self.pitch);
    }
}

pub struct Camera {
    pub position: Vec3,
    pub mode: CameraMode,
    pub follow_target: Option<u64>,
    pub death_linger: f32,
    pub linger_x: Option<f32>,
    pub linger_y: Option<f32>,
    pub world_size: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub freecam_speed: f32,
    pub freecam_controller: FreecamController,
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
            freecam_controller: FreecamController::default(),
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

    pub fn view_proj(&self) -> [[f32; 4]; 4] {
        coord::build_view_proj(
            self.position.x,
            self.position.y,
            self.position.z,
            self.yaw,
            self.pitch,
        )
    }

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

    pub fn world_to_screen(&self, wx: f32, wy: f32, wz: f32) -> Option<(f32, f32)> {
        let vp = self.view_proj();
        let w = frame::frame_width() as f32;
        let h = frame::frame_height() as f32;
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
