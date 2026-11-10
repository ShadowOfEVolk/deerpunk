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

    /// Fold a newer frame into this one. Resource deltas (meshes, textures,
    /// releases) of both frames are kept — the older frame's may not have been
    /// consumed yet — while the draw list and screen size come from the newer
    /// frame. Without this, producing frames faster than the render thread
    /// consumes them silently drops mesh/texture uploads, and every draw
    /// referencing them is skipped forever.
    pub fn absorb(&mut self, next: UiFrame) {
        for mesh in next.meshes {
            match self.meshes.iter_mut().find(|m| m.id == mesh.id) {
                Some(slot) => *slot = mesh,
                None => self.meshes.push(mesh),
            }
        }
        self.released_meshes.extend(next.released_meshes);
        let released_meshes = self.released_meshes.clone();
        self.meshes
            .retain(|m| !released_meshes.contains(&m.id));
        self.draws = next.draws;
        for (id, upload) in next.textures {
 self.textures.insert(id, upload);
        }
        for id in &next.released {
            self.textures.remove(id);
        }
        self.released.extend(next.released);
        self.screen = next.screen;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(id: u32) -> UiMesh {
        UiMesh {
            id,
            vertices: vec![UiVertex {
                pos: [0.0, 0.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            }],
            indices: vec![0],
        }
    }

    #[test]
    fn absorb_keeps_older_mesh_uploads_and_newer_draws() {
        let mut a = UiFrame {
            meshes: vec![mesh(2)],
            draws: vec![UiDraw {
                mesh_id: 2,
                translation: [1.0, 1.0],
                texture_id: 1,
                scissor: None,
            }],
            screen: [100, 100],
            ..Default::default()
        };
        let b = UiFrame {
            draws: vec![UiDraw {
                mesh_id: 3,
                translation: [2.0, 2.0],
                texture_id: 1,
                scissor: None,
            }],
            screen: [200, 200],
            ..Default::default()
        };
        a.absorb(b);
        assert_eq!(a.meshes.len(), 1);
        assert_eq!(a.meshes[0].id, 2, "unconsumed upload must survive");
        assert_eq!(a.draws.len(), 1);
        assert_eq!(a.draws[0].mesh_id, 3, "newer draw list wins");
        assert_eq!(a.screen, [200, 200]);
    }

    #[test]
    fn absorb_drops_mesh_released_before_consumption() {
        let mut a = UiFrame {
            meshes: vec![mesh(7)],
            ..Default::default()
        };
        let b = UiFrame {
            released_meshes: vec![7],
            ..Default::default()
        };
        a.absorb(b);
        assert!(a.meshes.is_empty(), "released-before-upload nets out");
        assert_eq!(a.released_meshes, vec![7]);
    }
 }
