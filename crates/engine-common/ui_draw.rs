//! GPU-ready UI draw list (RmlUi → renderer). No wgpu / RmlUi deps.

use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

/// One compiled mesh. Positions are local; [`UiDraw::translation`] places them.
#[derive(Clone, Debug)]
pub struct UiMesh {
    pub id: u32,
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
}

/// A draw of a previously compiled mesh. The renderer keeps the vertex buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiDraw {
    pub mesh_id: u32,
    pub translation: [f32; 2],
    pub texture_id: u32,
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

/// One frame of UI commands for the wgpu UI pass.
///
/// Meshes are uploaded once. Later frames only list draws unless a mesh was
/// compiled or released.
#[derive(Clone, Debug, Default)]
pub struct UiFrame {
    pub meshes: Vec<UiMesh>,
    pub released_meshes: Vec<u32>,
    pub draws: Vec<UiDraw>,
    /// New/updated textures this frame (id → RGBA8).
    pub textures: HashMap<u32, UiTextureUpload>,
    /// Texture ids released this frame.
    pub released: Vec<u32>,
    pub screen: [u32; 2],
}

impl UiFrame {
    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
            && self.meshes.is_empty()
            && self.released_meshes.is_empty()
            && self.textures.is_empty()
            && self.released.is_empty()
    }
}
