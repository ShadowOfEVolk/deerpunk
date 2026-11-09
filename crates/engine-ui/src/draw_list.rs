//! Accumulates RmlUi geometry into `UiFrame` (no CPU raster).
//!
//! `CompileGeometry` stores vertices once. `RenderGeometry` only records a draw
//! (mesh id, translation, texture, scissor). Identical draw lists produce no frame,
//! so a hover that does not change layout does not rebuild GPU meshes.

use std::collections::HashMap;

use deerpunk_engine_common::ui_draw::{UiDraw, UiFrame, UiMesh, UiTextureUpload, UiVertex};

struct StoredMesh {
    vertices: Vec<UiVertex>,
    indices: Vec<u32>,
}

pub struct UiGeometryBuilder {
    width: u32,
    height: u32,
    meshes: HashMap<u32, StoredMesh>,
    pending_meshes: Vec<u32>,
    released_meshes: Vec<u32>,
    draws: Vec<UiDraw>,
    last_draws: Vec<UiDraw>,
    next_mesh: u32,
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
            meshes: HashMap::new(),
            pending_meshes: Vec::new(),
            released_meshes: Vec::new(),
            draws: Vec::new(),
            last_draws: Vec::new(),
            next_mesh: 1,
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
        self.draws.clear();
        self.released.clear();
        self.released_meshes.clear();
        self.pending_meshes.clear();
        self.pending_uploads.clear();
    }

    /// `None` when this render produced the same draws and no new GPU resources.
    pub fn take_frame_if_changed(&mut self) -> Option<UiFrame> {
        let changed = !self.pending_meshes.is_empty()
            || !self.released_meshes.is_empty()
            || !self.pending_uploads.is_empty()
            || !self.released.is_empty()
            || self.draws != self.last_draws;
        if !changed {
            self.draws.clear();
            return None;
        }
        self.last_draws.clone_from(&self.draws);
        Some(self.take_frame())
    }

    fn take_frame(&mut self) -> UiFrame {
        let mut meshes = Vec::with_capacity(self.pending_meshes.len());
        for id in self.pending_meshes.drain(..) {
            if let Some(mesh) = self.meshes.get(&id) {
                meshes.push(UiMesh {
                    id,
                    vertices: mesh.vertices.clone(),
                    indices: mesh.indices.clone(),
                });
            }
        }
        UiFrame {
            meshes,
            released_meshes: std::mem::take(&mut self.released_meshes),
            draws: std::mem::take(&mut self.draws),
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

    pub fn load_texture_rgba(
        &mut self,
        rgba: Vec<u8>,
        w: u32,
        h: u32,
        filter_nearest: bool,
    ) -> u32 {
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

    /// Decode `Rml::Vertex` (position, premultiplied colour, uv) at `stride`.
    pub fn compile_geometry(
        &mut self,
        bytes: &[u8],
        stride: usize,
        num_vertices: usize,
        indices: &[i32],
    ) -> u32 {
        if stride < 20 || num_vertices == 0 || bytes.len() < stride * num_vertices {
            return 0;
        }
        let mut vertices = Vec::with_capacity(num_vertices);
        for i in 0..num_vertices {
            let v = &bytes[i * stride..];
            let x = f32::from_le_bytes(v[0..4].try_into().unwrap());
            let y = f32::from_le_bytes(v[4..8].try_into().unwrap());
            let u = f32::from_le_bytes(v[12..16].try_into().unwrap());
            let vv = f32::from_le_bytes(v[16..20].try_into().unwrap());
            vertices.push(UiVertex {
                pos: [x, y],
                uv: [u, vv],
                color: [v[8], v[9], v[10], v[11]],
            });
        }
        let mut idx = Vec::with_capacity(indices.len());
        for &i in indices {
            if i >= 0 && (i as usize) < num_vertices {
                idx.push(i as u32);
            }
        }
        let id = self.next_mesh;
        self.next_mesh = self.next_mesh.wrapping_add(1).max(1);
        self.meshes.insert(
            id,
            StoredMesh {
                vertices,
                indices: idx,
            },
        );
        self.pending_meshes.push(id);
        id
    }

    pub fn release_geometry(&mut self, id: u32) {
        if id == 0 {
            return;
        }
        self.meshes.remove(&id);
        if let Some(pos) = self.pending_meshes.iter().position(|m| *m == id) {
            self.pending_meshes.swap_remove(pos);
        } else {
            self.released_meshes.push(id);
        }
    }

    pub fn draw_geometry(&mut self, id: u32, tx: f32, ty: f32, texture: usize) {
        if id == 0 || !self.meshes.contains_key(&id) {
            return;
        }
        let tex_id = if texture == 0 || texture == usize::MAX {
            self.white
        } else {
            texture as u32
        };
        let scissor = if self.scissor_on { self.scissor } else { None };
        self.draws.push(UiDraw {
            mesh_id: id,
            translation: [tx, ty],
            texture_id: tex_id,
            scissor,
        });
    }
}
