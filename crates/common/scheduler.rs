use std::time::{Duration, Instant};

/// Планировщик кадров. Засекает время между кадрами, считает FPS.
pub struct FrameScheduler {
    pub dt: f32,
    pub fps: f32,
    pub max_fps: f32,
    last_frame: Instant,
}

impl FrameScheduler {
    /// `max_fps` — верхняя граница (по умолчанию 30).
    pub fn new(max_fps: f32) -> Self {
        Self {
            dt: 1.0 / max_fps,
            fps: max_fps,
            max_fps,
            last_frame: Instant::now(),
        }
    }

    /// Начать новый кадр. Возвращает `frame_secs` — реальное время с предыдущего кадра.
    pub fn begin_frame(&mut self) -> f32 {
        let now = Instant::now();
        let frame_time = now.duration_since(self.last_frame);
        self.last_frame = now;
        let frame_secs = frame_time.as_secs_f32().max(0.0001);
        self.fps += (1.0 / frame_secs - self.fps) * 0.1;
        frame_secs
    }

    /// Оставшееся время сна для поддержания max_fps.
    pub fn sleep_duration(&self) -> Duration {
        let elapsed = self.last_frame.elapsed();
        let budget = Duration::from_secs_f32(self.dt);
        if elapsed < budget {
            budget - elapsed
        } else {
            Duration::ZERO
        }
    }

    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Сменить FPS.
    pub fn set_max_fps(&mut self, fps: f32) {
        self.max_fps = fps;
        self.dt = 1.0 / fps;
    }
}
