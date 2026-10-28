use crate::resource::mesh_handle::MeshHandle;

/// Max UTF-8 bytes for [`Mesh3D::uri`] (NUL-padded, replicated on the wire).
pub const MESH3D_URI_MAX: usize = 256;

/// Static mesh: **URI** is authoritative on server / after net apply; **mesh** is client-local
/// [`MeshHandle`] resolved from the URI via the asset pipeline.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mesh3D {
    pub uri: [u8; MESH3D_URI_MAX],
    pub uri_len: u16,
    pub _pad: [u8; 6],
    pub mesh: MeshHandle,
}

impl Default for Mesh3D {
    fn default() -> Self {
        Self {
            uri: [0; MESH3D_URI_MAX],
            uri_len: 0,
            _pad: [0; 6],
            mesh: MeshHandle::INVALID,
        }
    }
}

impl Mesh3D {
    pub fn with_uri(uri: &str) -> Self {
        let mut m = Self::default();
        m.set_uri(uri);
        m
    }

    pub fn with_handle(handle: MeshHandle) -> Self {
        Self {
            uri: [0; MESH3D_URI_MAX],
            uri_len: 0,
            _pad: [0; 6],
            mesh: handle,
        }
    }

    pub fn set_uri(&mut self, uri: &str) {
        let bytes = uri.as_bytes();
        let n = bytes.len().min(MESH3D_URI_MAX);
        self.uri[..n].copy_from_slice(&bytes[..n]);
        if n < MESH3D_URI_MAX {
            self.uri[n..].fill(0);
        }
        self.uri_len = n as u16;
    }

    pub fn uri_str(&self) -> &str {
        let n = (self.uri_len as usize).min(MESH3D_URI_MAX);
        std::str::from_utf8(&self.uri[..n]).unwrap_or("")
    }
}
