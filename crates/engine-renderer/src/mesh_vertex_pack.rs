//! Pack [`Mesh`](deerpunk_engine_common::resource::mesh::Mesh) CPU arrays into GPU vertex bytes
//! using a shader [`VertexLayoutSpec`].

use deerpunk_engine_common::coord::UP;
use deerpunk_engine_common::material::{MeshVertexChannel, VertexAttrType, VertexLayoutSpec};
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::resource::mesh_compact::{
    encode_baked_color_unorm8x4, encode_light_tile_u32, encode_position_u20x3,
    encode_uv_packed_u16x2,
};

pub fn wgpu_vertex_format(ty: VertexAttrType) -> wgpu::VertexFormat {
    match ty {
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
    if mesh.packed_stride > 0 {
        assert_eq!(
            mesh.packed_stride, spec.stride,
            "mesh packed_stride {} != shader layout stride {}",
            mesh.packed_stride, spec.stride
        );
        assert!(
            mesh.uses_prepacked_vertices(),
            "compact mesh (packed_stride={}) must have full vertex_bytes; no SoA repack fallback",
            mesh.packed_stride
        );
        return mesh.vertex_bytes.clone();
    }
    let n = mesh.vertex_count();
    let stride = spec.stride as usize;
    let mut out = vec![0u8; n * stride];
    for i in 0..n {
        let base = i * stride;
        for attr in &spec.attrs {
            write_attr(
                &mut out[base + attr.offset as usize..],
                mesh,
                i,
                attr.channel,
                attr.ty,
            );
        }
    }
    out
}

fn mesh_atlas_tile(mesh: &Mesh, i: usize) -> u16 {
    mesh.atlas_tile.get(i).copied().unwrap_or(0)
}

fn write_attr(
    dst: &mut [u8],
    mesh: &Mesh,
    i: usize,
    channel: MeshVertexChannel,
    ty: VertexAttrType,
) {
    match (channel, ty) {
        (MeshVertexChannel::Position, VertexAttrType::PackedU20X3) => {
            let w = encode_position_u20x3(mesh.positions[i]);
            dst[..4].copy_from_slice(&w[0].to_le_bytes());
            dst[4..8].copy_from_slice(&w[1].to_le_bytes());
        }
        (MeshVertexChannel::Position, _) => {
            write_f32_slice(dst, &mesh.positions[i]);
        }
        (MeshVertexChannel::Normal, _) => {
            let v = mesh.normals.get(i).copied().unwrap_or(UP);
            write_f32_slice(dst, &v);
        }
        (MeshVertexChannel::Uv, VertexAttrType::PackedU16X2) => {
            let uv = mesh.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            let w = encode_uv_packed_u16x2(uv);
            dst[..4].copy_from_slice(&w.to_le_bytes());
        }
        (MeshVertexChannel::Uv, _) => {
            let v = mesh.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            write_f32_slice(dst, &v);
        }
        (MeshVertexChannel::AtlasRect, _) => {
            let v = mesh
                .atlas_rects
                .get(i)
                .copied()
                .unwrap_or([0.0, 0.0, 1.0, 1.0]);
            write_f32_slice(dst, &v);
        }
        (MeshVertexChannel::ExtFloat2_0, _) => {
            let v = mesh.lightmap_uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            write_f32_slice(dst, &v);
        }
        (MeshVertexChannel::ExtFloat4_0, _) => {
            let v = mesh
                .lightmap_styles
                .get(i)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            write_f32_slice(dst, &v);
        }
        (MeshVertexChannel::BakedColor, VertexAttrType::Unorm8X4) => {
            let s = mesh
                .lightmap_styles
                .get(i)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let c = encode_baked_color_unorm8x4([s[0], s[1], s[2]], s[3]);
            dst[..4].copy_from_slice(&c);
        }
        (MeshVertexChannel::LightTile, VertexAttrType::LightTileU32) => {
            let lm = mesh.lightmap_uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            let tile = mesh_atlas_tile(mesh, i);
            let w = encode_light_tile_u32(lm[0], lm[1], tile);
            dst[..4].copy_from_slice(&w.to_le_bytes());
        }
        (MeshVertexChannel::Joints, _) => {
            let ji = mesh.joint_indices.get(i).copied().unwrap_or([0, 0, 0, 0]);
            let words = [
                ji[0] as u32,
                ji[1] as u32,
                ji[2] as u32,
                ji[3] as u32,
            ];
            dst[..16].copy_from_slice(bytemuck::cast_slice(&words));
        }
        (MeshVertexChannel::Weights, _) => {
            let w = mesh.joint_weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]);
            write_f32_slice(dst, &w);
        }
        _ => {}
    }
}

fn write_f32_slice(dst: &mut [u8], values: &[f32]) {
    dst[..values.len() * 4].copy_from_slice(bytemuck::cast_slice(values));
}