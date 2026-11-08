//! GPU vertex bytes are produced by the game. The engine checks `vertex_size`
//! against the shader `#@vertex` stride and uploads `vertex_bytes` unchanged.

use deerpunk_engine_common::material::{VertexAttrType, VertexLayoutSpec};
use deerpunk_engine_common::resource::mesh::Mesh;

pub fn wgpu_vertex_format(ty: VertexAttrType) -> wgpu::VertexFormat {
    match ty {
        VertexAttrType::Uint32 => wgpu::VertexFormat::Uint32,
        VertexAttrType::Float2 => wgpu::VertexFormat::Float32x2,
        VertexAttrType::Float3 => wgpu::VertexFormat::Float32x3,
        VertexAttrType::Float4 | VertexAttrType::Weights => wgpu::VertexFormat::Float32x4,
        VertexAttrType::Joints => wgpu::VertexFormat::Uint32x4,
        VertexAttrType::PackedU20X3 => wgpu::VertexFormat::Uint32x2,
        VertexAttrType::Unorm8X4 => wgpu::VertexFormat::Unorm8x4,
        VertexAttrType::PackedU16X2 | VertexAttrType::LightTileU32 => wgpu::VertexFormat::Uint32,
    }
}

pub fn pack_mesh_vertices(mesh: &Mesh, spec: &VertexLayoutSpec) -> Vec<u8> {
    assert!(
        mesh.vertex_size > 0 && mesh.uses_prepacked_vertices(),
        "mesh GPU upload requires vertex_bytes and vertex_size"
    );
    assert_eq!(
        mesh.vertex_size, spec.stride,
        "mesh vertex_size {} != shader layout stride {}",
        mesh.vertex_size, spec.stride
    );
    mesh.vertex_bytes.clone()
}
