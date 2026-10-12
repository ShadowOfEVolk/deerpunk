use tiny_skia::Color;

use deerpunk_engine_common::frame::{frame_height, frame_width};

use crate::font::{push_char, push_text, push_text_centered_shadow, text_width_f32};
use crate::snapshot::{RenderSnapshot, rgba_u8_4};

const GLYPH_SCALE: f32 = 2.0;
const SHADOW_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 200.0 / 255.0];

pub fn color_f32(c: Color) -> [f32; 4] {
    [
        c.red() as f32,
        c.green() as f32,
        c.blue() as f32,
        c.alpha() as f32,
    ]
}

pub fn push_text_centered_parts(
    snap: &mut RenderSnapshot,
    cx: f32,
    y: f32,
    parts: &[(&str, Color)],
) {
    let total_w: f32 = parts
        .iter()
        .map(|(s, _)| text_width_f32(s, GLYPH_SCALE))
        .sum();
    let mut x = cx - total_w * 0.5;
    for (text, color) in parts {
        let c = color_f32(*color);
        push_text(snap, x + 1.0, y + 1.0, text, SHADOW_COLOR, GLYPH_SCALE);
        push_text(snap, x, y, text, c, GLYPH_SCALE);
        x += text_width_f32(text, GLYPH_SCALE);
    }
}

pub fn push_text_centered(snap: &mut RenderSnapshot, cx: f32, y: f32, text: &str, color: Color) {
    push_text_centered_shadow(snap, cx, y, text, color_f32(color), GLYPH_SCALE);
}

pub fn push_text_centered_f32(
    snap: &mut RenderSnapshot,
    cx: f32,
    y: f32,
    text: &str,
    color: Color,
    scale: f32,
) {
    let w = text_width_f32(text, scale);
    let c = color_f32(color);
    push_text(snap, cx - w * 0.5 + 1.0, y + 1.0, text, SHADOW_COLOR, scale);
    push_text(snap, cx - w * 0.5, y, text, c, scale);
}

/// Прямоугольник-заливка (HUD примитив).
pub fn push_rect(snap: &mut RenderSnapshot, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
    snap.push_hud(crate::snapshot::Instance::solid(
        x + w * 0.5,
        y + h * 0.5,
        w * 0.5,
        h * 0.5,
        color,
    ));
}

/// Древовидный debug-вывод gpu-примитива (для отладки шейдера).
#[allow(dead_code)]
pub(crate) fn debug_push_text(snap: &mut RenderSnapshot, x: f32, y: f32, text: &str) {
    for (i, byte) in text.bytes().enumerate() {
        let color = if (i as u32 % 2) == 0 {
            rgba_u8_4([255, 255, 255, 255])
        } else {
            rgba_u8_4([200, 200, 200, 255])
        };
        push_char(snap, x + i as f32 * 8.0, y, byte, color, 1.0);
    }
}
