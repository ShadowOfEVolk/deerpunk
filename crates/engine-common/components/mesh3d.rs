use crate::components::render_layer::RENDER_LAYER_DEFAULT_DRAW;
use crate::resource::mesh_handle::MeshHandle;

/// Max UTF-8 bytes for [`Mesh3D::uri`] (NUL-padded, replicated on the wire).
pub const MESH3D_URI_MAX: usize = 256;

/// Static mesh: **URI** is authoritative on server / after net apply; **mesh** is client-local
/// [`MeshHandle`] resolved from the URI via the asset pipeline.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mesh3D {
    pub uri: [u8; MESH3D_URI_MAX],
    pub uri_len: u16,
    pub _pad: u16,
    /// Camera bits this mesh is drawn on. See [`crate::components::render_layer`].
    pub layer_mask: u32,
    pub mesh: MeshHandle,
    /// Object-space AABB copied at mesh assign. Readers do not scan vertices.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// World AABB baked for [`Self::baked_pos`] / rotation / scale.
    pub world_min: [f32; 3],
    pub world_max: [f32; 3],
    pub baked_pos: [f32; 3],
    pub baked_rot: [f32; 4],
    pub baked_scale: [f32; 3],
    /// `0` until [`Self::bake_world`].
    pub world_baked: u32,
}

impl Default for Mesh3D {
    fn default() -> Self {
        Self {
            uri: [0; MESH3D_URI_MAX],
            uri_len: 0,
            _pad: 0,
            layer_mask: RENDER_LAYER_DEFAULT_DRAW,
            mesh: MeshHandle::INVALID,
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            world_min: [0.0; 3],
            world_max: [0.0; 3],
            baked_pos: [0.0; 3],
            baked_rot: [0.0; 4],
            baked_scale: [0.0; 3],
            world_baked: 0,
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
            _pad: 0,
            layer_mask: RENDER_LAYER_DEFAULT_DRAW,
            mesh: handle,
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            world_min: [0.0; 3],
            world_max: [0.0; 3],
            baked_pos: [0.0; 3],
            baked_rot: [0.0; 4],
            baked_scale: [0.0; 3],
            world_baked: 0,
        }
    }

    pub fn set_local_bounds(mut self, min: [f32; 3], max: [f32; 3]) -> Self {
        self.bounds_min = min;
        self.bounds_max = max;
        self.world_baked = 0;
        self
    }

    pub fn has_local_bounds(&self) -> bool {
        self.bounds_min[0] <= self.bounds_max[0]
            && self.bounds_min[1] <= self.bounds_max[1]
            && self.bounds_min[2] <= self.bounds_max[2]
    }

    pub fn fresh_world(
        &self,
        pos: [f32; 3],
        rot: [f32; 4],
        scale: [f32; 3],
    ) -> Option<([f32; 3], [f32; 3])> {
        if self.world_baked == 0
            || self.baked_pos != pos
            || self.baked_rot != rot
            || self.baked_scale != scale
        {
            return None;
        }
        Some((self.world_min, self.world_max))
    }

    pub fn bake_world(
        &mut self,
        pos: [f32; 3],
        rot: [f32; 4],
        scale: [f32; 3],
        world_min: [f32; 3],
        world_max: [f32; 3],
    ) {
        self.baked_pos = pos;
        self.baked_rot = rot;
        self.baked_scale = scale;
        self.world_min = world_min;
        self.world_max = world_max;
        self.world_baked = 1;
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

    pub fn with_layer_mask(mut self, mask: u32) -> Self {
        self.layer_mask = mask;
        self
    }

    pub fn uri_str(&self) -> &str {
        let n = (self.uri_len as usize).min(MESH3D_URI_MAX);
        std::str::from_utf8(&self.uri[..n]).unwrap_or("")
    }
}
