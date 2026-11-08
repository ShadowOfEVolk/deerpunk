//! ECS component for material assignment.
//!
//! **URI** is authoritative on the server / after net apply; **handle** is resolved on the
//! client via `MaterialStore` (same pattern as [`crate::components::Mesh3D`]).

use crate::material::handle::MaterialHandle;

/// Max UTF-8 bytes for [`DrawMaterial::uri`].
pub const DRAW_MATERIAL_URI_MAX: usize = 256;

/// ECS component: material asset reference for rendering.
///
/// Attach alongside `TransformComponent + Mesh3D`. `handle == 0` → default material until resolved.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrawMaterial {
    pub uri: [u8; DRAW_MATERIAL_URI_MAX],
    pub uri_len: u16,
    pub _pad: u16,
    /// Logical material asset ([`deerpunk_engine_resource::AssetId`]). Client-only; 0 = use [`Self::uri`].
    pub asset_id: u64,
    /// GPU row in client `MaterialStore` (resolved from `asset_id` / `uri`).
    pub handle: MaterialHandle,
    /// Crack overlay stage 0..9. Client-only. `block_break.wgsl` reads `material_data.y`.
    pub crack_stage: u32,
    /// Chunk pop-in spawn time as `f32` bits. Client-only. Non-zero wins over [`Self::crack_stage`].
    pub reveal_spawn_bits: u32,
}

impl Default for DrawMaterial {
    fn default() -> Self {
        Self {
            uri: [0; DRAW_MATERIAL_URI_MAX],
            uri_len: 0,
            _pad: 0,
            asset_id: 0,
            handle: MaterialHandle::NONE,
            crack_stage: 0,
            reveal_spawn_bits: 0,
        }
    }
}

impl DrawMaterial {
    pub fn with_uri(uri: &str) -> Self {
        let mut m = Self::default();
        m.set_uri(uri);
        m
    }

    pub fn with_handle(handle: MaterialHandle) -> Self {
        Self {
            uri: [0; DRAW_MATERIAL_URI_MAX],
            uri_len: 0,
            _pad: 0,
            asset_id: 0,
            handle,
            crack_stage: 0,
            reveal_spawn_bits: 0,
        }
    }

    pub fn new(handle: MaterialHandle) -> Self {
        Self::with_handle(handle)
    }

    pub fn none() -> Self {
        Self::default()
    }

    pub fn set_uri(&mut self, uri: &str) {
        let bytes = uri.as_bytes();
        let n = bytes.len().min(DRAW_MATERIAL_URI_MAX);
        self.uri[..n].copy_from_slice(&bytes[..n]);
        if n < DRAW_MATERIAL_URI_MAX {
            self.uri[n..].fill(0);
        }
        self.uri_len = n as u16;
    }

    pub fn set_asset_id(&mut self, id: u64) {
        self.asset_id = id;
    }

    pub fn asset_id(&self) -> u64 {
        self.asset_id
    }

    /// Value copied into `MeshUniform.material_data.y`.
    pub fn material_data_y(self) -> u32 {
        if self.reveal_spawn_bits != 0 {
            self.reveal_spawn_bits
        } else {
            self.crack_stage
        }
    }

    pub fn set_crack_stage(&mut self, stage: u8) {
        self.crack_stage = u32::from(stage.min(9));
        self.reveal_spawn_bits = 0;
    }

    pub fn set_chunk_reveal(&mut self, spawn_time: f32) {
        self.crack_stage = 0;
        self.reveal_spawn_bits = spawn_time.to_bits();
    }

    pub fn uri_str(&self) -> &str {
        let n = (self.uri_len as usize).min(DRAW_MATERIAL_URI_MAX);
        std::str::from_utf8(&self.uri[..n]).unwrap_or("")
    }
}
