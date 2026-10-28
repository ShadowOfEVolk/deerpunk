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
    pub _pad: [u8; 6],
    pub handle: MaterialHandle,
}

impl Default for DrawMaterial {
    fn default() -> Self {
        Self {
            uri: [0; DRAW_MATERIAL_URI_MAX],
            uri_len: 0,
            _pad: [0; 6],
            handle: MaterialHandle::NONE,
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
            _pad: [0; 6],
            handle,
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

    pub fn uri_str(&self) -> &str {
        let n = (self.uri_len as usize).min(DRAW_MATERIAL_URI_MAX);
        std::str::from_utf8(&self.uri[..n]).unwrap_or("")
    }
}
