//! One chunk, two mesh slots. A `renderer.prepare` system submits these;
//! they are not extracted as `Mesh3D`.

use crate::resource::mesh_handle::MeshHandle;

/// Client chunk draw. Handles are registry ids; `0` means that slot is empty.
///
/// `opaque` holds every compact layer (opaque, plants, foliage) as index ranges.
/// `transparent` and `cutout` stay in the layout so older chunks can still pin a
/// mesh per layer; new uploads leave them invalid. `fluid` is a separate mesh
/// because its vertex layout is not the compact 20-byte stride.
///
/// `hidden != 0` skips the chunk (cave cull). `reveal_spawn_bits` is the pop-in
/// start time (`f32` bits); `0` means the settled materials.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRenderer {
    pub opaque: MeshHandle,
    pub transparent: MeshHandle,
    pub cutout: MeshHandle,
    pub fluid: MeshHandle,
    pub reveal_spawn_bits: u32,
    pub hidden: u8,
    pub _pad: [u8; 3],
    /// Index range per layer (opaque, transparent, cutout, fluid) and face.
    /// Face 0 is non-axis geometry (always drawn). Faces 1..=6 are -X +X -Y +Y -Z +Z.
    pub face_first: [[u32; 7]; 4],
    pub face_count: [[u32; 7]; 4],
}

impl Default for ChunkRenderer {
    fn default() -> Self {
        Self {
            opaque: MeshHandle::INVALID,
            transparent: MeshHandle::INVALID,
            cutout: MeshHandle::INVALID,
            fluid: MeshHandle::INVALID,
            reveal_spawn_bits: 0,
            hidden: 0,
            _pad: [0; 3],
            face_first: [[0; 7]; 4],
            face_count: [[0; 7]; 4],
        }
    }
}

impl ChunkRenderer {
    pub fn handles(self) -> [MeshHandle; 4] {
        [self.opaque, self.transparent, self.cutout, self.fluid]
    }

    pub fn any_mesh(self) -> bool {
        self.handles().iter().any(|h| h.is_valid())
    }
}
