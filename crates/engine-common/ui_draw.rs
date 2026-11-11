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
    ///
    /// Only fold frames that have NOT reached the renderer's UI pass yet: once
    /// consumed, this frame's resource entries are already in the GPU caches
    /// and folding would re-carry them, making every later upload replay the
    /// whole accumulated set. Callers that can observe consumption (the
    /// renderer's upload flag) should use [`Self::absorb_delta`] instead.
    pub fn absorb(&mut self, next: UiFrame) {
        for mesh in next.meshes {
            match self.meshes.iter_mut().find(|m| m.id == mesh.id) {
                Some(slot) => *slot = mesh,
                None => self.meshes.push(mesh),
            }
        }
        self.released_meshes.extend(next.released_meshes);
        let released_meshes = self.released_meshes.clone();
        self.meshes.retain(|m| !released_meshes.contains(&m.id));
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

    /// Merge a newer frame into this one, retiring already-consumed resources.
    ///
    /// When `prev_consumed` is `true` the renderer's UI pass already uploaded
    /// this frame's resource entries into its GPU caches, so they are dropped
    /// before folding — the merge carries only `next`'s delta and the next
    /// upload replays just what changed. When `false` this behaves exactly
    /// like [`Self::absorb`] (older deltas kept, nothing lost).
    pub fn absorb_delta(&mut self, next: UiFrame, prev_consumed: bool) {
        if prev_consumed {
            self.meshes.clear();
            self.released_meshes.clear();
            self.textures.clear();
            self.released.clear();
        }
        self.absorb(next);
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

    fn tex_upload(id: u32) -> (u32, UiTextureUpload) {
        (
            id,
            UiTextureUpload {
                width: 4,
                height: 4,
                rgba: vec![0; 4 * 4 * 4],
                filter_nearest: false,
            },
        )
    }

    /// The per-frame upload contract: once the renderer's UI pass consumed a
    /// frame, folding the next delta must retire the consumed entries so the
    /// upload only carries what actually changed (otherwise every frame
    /// replays the whole accumulated mesh/texture set).
    #[test]
    fn absorb_delta_retires_consumed_resources() {
        let mut pending = UiFrame {
            meshes: vec![mesh(1), mesh(2)],
            textures: [tex_upload(9)].into_iter().collect(),
            draws: vec![UiDraw {
                mesh_id: 1,
                translation: [0.0, 0.0],
                texture_id: 9,
                scissor: None,
            }],
            released_meshes: vec![3],
            released: vec![4],
            screen: [100, 100],
            ..Default::default()
        };
        // UI pass consumed `pending`; a new frame carries one changed mesh.
        let next = UiFrame {
            meshes: vec![mesh(5)],
            draws: vec![UiDraw {
                mesh_id: 5,
                translation: [0.0, 0.0],
                texture_id: 9,
                scissor: None,
            }],
            screen: [100, 100],
            ..Default::default()
        };
        pending.absorb_delta(next, true);
        assert_eq!(
            pending.meshes.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![5],
            "consumed entries must not be re-uploaded"
        );
        assert!(pending.textures.is_empty(), "consumed textures retired");
        assert!(
            pending.released_meshes.is_empty(),
            "consumed releases retired"
        );
        assert!(pending.released.is_empty());
        assert_eq!(pending.draws.len(), 1, "newer draw list wins");
        assert_eq!(pending.draws[0].mesh_id, 5);
    }

    /// Without a consumption signal the old absorb semantics must hold: older
    /// deltas survive so a frame produced while the UI pass was not reached is
    /// never lost.
    #[test]
    fn absorb_delta_keeps_unconsumed_resources() {
        let mut pending = UiFrame {
            meshes: vec![mesh(1)],
            textures: [tex_upload(9)].into_iter().collect(),
            screen: [100, 100],
            ..Default::default()
        };
        let next = UiFrame {
            meshes: vec![mesh(5)],
            draws: vec![UiDraw {
                mesh_id: 5,
                translation: [0.0, 0.0],
                texture_id: 9,
                scissor: None,
            }],
            screen: [100, 100],
            ..Default::default()
        };
        pending.absorb_delta(next, false);
        assert_eq!(
            pending.meshes.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 5],
            "unconsumed upload must survive"
        );
        assert_eq!(pending.textures.len(), 1);
        assert_eq!(pending.draws[0].mesh_id, 5);
    }
}
