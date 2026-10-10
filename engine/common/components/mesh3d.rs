use std::sync::Arc;

use crate::resource::mesh::Mesh;

/// Mesh3D component — holds mesh reference (`Arc` so extract can share without cloning verts).
///
/// Pairs with Material to form a renderable entity.
/// The renderer's MeshAllocator uploads the mesh to GPU and returns a MeshHandle.
#[repr(C)]
#[derive(Clone)]
pub struct Mesh3D {
    pub mesh: Option<Arc<Mesh>>,
}

impl Mesh3D {
    pub fn new(mesh: Mesh) -> Self {
        Self {
            mesh: Some(Arc::new(mesh)),
        }
    }

    pub fn with_mesh(mesh: Mesh) -> Self {
        Self::new(mesh)
    }

    pub fn with_arc(mesh: Arc<Mesh>) -> Self {
        Self { mesh: Some(mesh) }
    }

    pub fn cube() -> Self {
        Self::new(Mesh::cube())
    }
}

impl Default for Mesh3D {
    fn default() -> Self {
        Self { mesh: None }
    }
}
