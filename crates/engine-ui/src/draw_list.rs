//! Accumulates RmlUi geometry into `UiFrame` (no CPU raster).

use std::collections::HashMap;

use deerpunk_engine_common::ui_draw::{UiBatch, UiFrame, UiTextureUpload, UiVertex};

pub struct UiGeometryBuilder {
    width: u32,
    height: u32,
    vertices: Vec<UiVertex>,
    indices: Vec<u32>,
    batches: Vec<UiBatch>,
    textures: HashMap<u32, UiTextureUpload>,
    released: Vec<u32>,
    next_tex: u32,
    white: u32,
    scissor: Option<[i32; 4]>,
    scissor_on: bool,
    /// Pending texture uploads only (cleared when frame is taken).
    pending_uploads: HashMap<u32, UiTextureUpload>,
}

impl UiGeometryBuilder {
    pub fn new(width: u32, height: u32) -> Self {
        let white = 1;
        let mut textures = HashMap::new();
        textures.insert(
            white,
            UiTextureUpload {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
                filter_nearest: false,
            },
        );
        let mut pending = HashMap::new();
        pending.insert(white, textures[&white].clone());
        Self {
            width,
            height,
            vertices: Vec::new(),
            indices: Vec::new(),
            batches: Vec::new(),
            textures,
            released: Vec::new(),
            next_tex: 2,
            white,
            scissor: None,
            scissor_on: false,
            pending_uploads: pending,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn begin_frame(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.batches.clear();
        self.released.clear();
        self.pending_uploads.clear();
    }

    /// Ensure the 1x1 white texture is uploaded at least once (solid-color geometry).
    pub fn ensure_white_upload(&mut self) {
        if let Some(w) = self.textures.get(&self.white).cloned() {
            self.pending_uploads.entry(self.white).or_insert(w);
        }
    }

    pub fn take_frame(&mut self) -> UiFrame {
        UiFrame {
            vertices: std::mem::take(&mut self.vertices),
            indices: std::mem::take(&mut self.indices),
            batches: std::mem::take(&mut self.batches),
            textures: std::mem::take(&mut self.pending_uploads),
            released: std::mem::take(&mut self.released),
            screen: [self.width, self.height],
        }
    }

    pub fn white_id(&self) -> u32 {
        self.white
    }

    pub fn generate_texture(&mut self, rgba: &[u8], w: i32, h: i32) -> u32 {
        let id = self.next_tex;
        self.next_tex += 1;
        let upload = UiTextureUpload {
            width: w as u32,
            height: h as u32,
            rgba: rgba.to_vec(),
            filter_nearest: false,
        };
        self.textures.insert(id, upload.clone());
        self.pending_uploads.insert(id, upload);
        id
    }

    pub fn load_texture_rgba(&mut self, rgba: Vec<u8>, w: u32, h: u32, filter_nearest: bool) -> u32 {
        let id = self.next_tex;
        self.next_tex += 1;
        let upload = UiTextureUpload {
            width: w,
            height: h,
            rgba,
            filter_nearest,
        };
        self.textures.insert(id, upload.clone());
        self.pending_uploads.insert(id, upload);
        id
    }

    pub fn release_texture(&mut self, id: u32) {
        if id != self.white {
            self.textures.remove(&id);
            self.released.push(id);
        }
    }

    pub fn set_scissor(&mut self, enable: bool) {
        self.scissor_on = enable;
    }

    pub fn set_scissor_region(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.scissor = Some([x, y, w, h]);
    }

    pub fn draw_indexed(
        &mut self,
        positions: &[f32],
        colours: &[u8],
        texcoords: &[f32],
        indices: &[i32],
        tx: f32,
        ty: f32,
        texture: usize,
    ) {
        let tex_id = if texture == 0 || texture == usize::MAX {
            self.white
        } else {
            texture as u32
        };

        let base = self.vertices.len() as u32;
        let max_v = indices.iter().copied().max().unwrap_or(0) as usize + 1;
        for i in 0..max_v {
            if i * 2 + 1 >= positions.len() {
                break;
            }
            self.vertices.push(UiVertex {
                pos: [positions[i * 2] + tx, positions[i * 2 + 1] + ty],
                uv: [
                    texcoords.get(i * 2).copied().unwrap_or(0.0),
                    texcoords.get(i * 2 + 1).copied().unwrap_or(0.0),
                ],
                color: [
                    colours.get(i * 4).copied().unwrap_or(255),
                    colours.get(i * 4 + 1).copied().unwrap_or(255),
                    colours.get(i * 4 + 2).copied().unwrap_or(255),
                    colours.get(i * 4 + 3).copied().unwrap_or(255),
                ],
            });
        }

        let index_start = self.indices.len() as u32;
        for &idx in indices {
            self.indices.push(base + idx as u32);
        }
        let index_count = self.indices.len() as u32 - index_start;

        let scissor = if self.scissor_on { self.scissor } else { None };

        // Merge with previous batch if same texture + scissor
        if let Some(last) = self.batches.last_mut() {
            if last.texture_id == tex_id && last.scissor == scissor {
                last.index_count += index_count;
                return;
            }
        }

        self.batches.push(UiBatch {
            texture_id: tex_id,
            index_start,
            index_count,
            scissor,
        });
    }
}
