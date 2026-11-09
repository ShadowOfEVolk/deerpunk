//! C layout for one `v1:client:submit`. Transform is shared. Each binset is one
//! indirect batch (one mesh, one material). Bins are the commands inside it.

/// One indirect command. Same mesh and material as the other bins in its binset.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MeshSubmitBin {
    pub first_index: u32,
    pub index_count: u32,
    pub face: u8,
    pub _pad: [u8; 3],
}

/// One multi-draw. `bin_start` indexes the shared bin array on [`MeshSubmit`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MeshSubmitBinset {
    pub mesh: u64,
    pub material: u32,
    pub uniform_aux: u32,
    pub bin_start: u32,
    pub bin_count: u32,
}

/// One prepare submit.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MeshSubmit {
    pub entity: u64,
    pub layer_mask: u32,
    pub binset_count: u32,
    pub position: [f32; 3],
    pub _pad_pos: f32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub bin_count: u32,
    /// Non-zero: caller already tested this submit against the view frustum.
    pub skip_vis: u32,
    /// Aligns the pointers below.
    pub _pad_tail: u32,
    pub binsets: *const MeshSubmitBinset,
    pub bins: *const MeshSubmitBin,
}
