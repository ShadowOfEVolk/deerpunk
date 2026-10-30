use std::sync::atomic::{AtomicU32, Ordering};

static FRAME_W: AtomicU32 = AtomicU32::new(0);
static FRAME_H: AtomicU32 = AtomicU32::new(0);

/// Размер кадра (16:9). Env: `FRAME_WIDTH`/`FRAME_HEIGHT`, `RTMP_RESOLUTION`, `RTMP_WIDTH`/`RTMP_HEIGHT`.
pub fn output_frame_size() -> (u32, u32) {
    if let (Ok(w), Ok(h)) = (std::env::var("FRAME_WIDTH"), std::env::var("FRAME_HEIGHT")) {
        if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
            if w > 0 && h > 0 && w.is_multiple_of(2) && h.is_multiple_of(2) {
                return (w, h);
            }
        }
    }

    if let (Ok(w), Ok(h)) = (std::env::var("RTMP_WIDTH"), std::env::var("RTMP_HEIGHT")) {
        if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
            if w > 0 && h > 0 && w.is_multiple_of(2) && h.is_multiple_of(2) {
                return (w, h);
            }
        }
    }

    match std::env::var("RTMP_RESOLUTION")
        .or_else(|_| std::env::var("FRAME_RESOLUTION"))
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "720p" | "720" => (1280, 720),
        "1080p" | "1080" => (1920, 1080),
        "360p" | "360" => (640, 360),
        "480p" | "480" => (854, 480),
        _ => (1280, 720),
    }
}

fn ensure_frame_initialized() {
    if FRAME_W.load(Ordering::Relaxed) == 0 {
        let (w, h) = output_frame_size();
        set_frame_size(w, h);
        eprintln!("[frame] {w}x{h}");
    }
}

/// Размер буфера рендера (= размер окна / TCP-кадра при стриме).
pub fn frame_size() -> (u32, u32) {
    ensure_frame_initialized();
    (
        FRAME_W.load(Ordering::Relaxed),
        FRAME_H.load(Ordering::Relaxed),
    )
}

/// Обновить размер кадра (resize окна, shared viewport). Вызывать до `resize_framebuffer`.
pub fn set_frame_size(width: u32, height: u32) {
    let w = width.max(1);
    let h = height.max(1);
    FRAME_W.store(w, Ordering::Relaxed);
    FRAME_H.store(h, Ordering::Relaxed);
}

pub fn frame_width() -> u32 {
    frame_size().0
}

pub fn frame_height() -> u32 {
    frame_size().1
}

/// Пикселей на метр по вертикали (как раньше `screen_size / view_size`).
pub fn frame_ppm(view_size: f32) -> f32 {
    frame_height() as f32 / view_size.max(1.0)
}

pub fn format_lifetime(secs: f32) -> String {
    let total = secs.max(0.0).round() as u32;
    let minutes = total / 60;
    let seconds = total % 60;
    if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Размеры танка (используются рендерером).
pub const TANK_WIDTH: f32 = 2.0;
pub const TANK_LENGTH: f32 = 3.0;
