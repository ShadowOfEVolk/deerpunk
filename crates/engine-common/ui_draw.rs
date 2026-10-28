//! GPU-ready UI draw list (RmlUi → renderer). No wgpu / RmlUi deps.

use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct UiBatch {
    pub texture_id: u32,
    pub index_start: u32,
    pub index_count: u32,
    pub scissor: Option<[i32; 4]>,
}

#[derive(Clone, Debug, Default)]
pub struct UiTextureUpload {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Pixel-art HUD icons: nearest-neighbor sampling when scaling.
    pub filter_nearest: bool,
}

/// One frame of UI geometry for the wgpu UI pass.
#[derive(Clone, Debug, Default)]
pub struct UiFrame {
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<UiBatch>,
    /// New/updated textures this frame (id → RGBA8).
    pub textures: HashMap<u32, UiTextureUpload>,
    /// Texture ids released this frame.
    pub released: Vec<u32>,
    pub screen: [u32; 2],
}

impl UiFrame {
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }
}
