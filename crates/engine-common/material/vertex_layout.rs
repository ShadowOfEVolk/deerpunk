//! Shader-declared mesh vertex layouts (`#@vertex` in WGSL).
//!
//! Channel names map to optional [`Mesh`](crate::resource::mesh::Mesh) arrays — the engine
//! does not assign game-specific meaning (no "voxel" / "foliage" types).

use std::hash::{Hash, Hasher};

/// WGSL scalar layout for one vertex attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VertexAttrType {
    /// Single `u32` (`Uint32` in WGSL) — e.g. per-vertex tag bits.
    Uint32,
    Float2,
    Float3,
    Float4,
    /// Skinned meshes: four joint indices (`Uint32x4` in the vertex buffer).
    Joints,
    /// Skinned meshes: four weights (`Float32x4`).
    Weights,
    /// Two `u32`: interleaved 20-bit X/Y/Z (8 bytes).
    PackedU20X3,
    /// Four UNORM8 (tint×shade baked; A=255 unless material needs otherwise).
    Unorm8X4,
    /// Face UV 0..1 as two 16-bit values in one `u32`.
    PackedU16X2,
    /// Upper 16 bits: block/sky light bytes; lower 16: atlas tile index.
    LightTileU32,
}

impl VertexAttrType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "float2" | "vec2" => Some(Self::Float2),
            "float3" | "vec3" => Some(Self::Float3),
            "float4" | "vec4" => Some(Self::Float4),
            "joints" | "uint4" | "u32x4" => Some(Self::Joints),
            "weights" | "weight4" => Some(Self::Weights),
            "packed_u20x3" | "u20x3" => Some(Self::PackedU20X3),
            "unorm8x4" | "rgba8" => Some(Self::Unorm8X4),
            "packed_u16x2" | "uv16" => Some(Self::PackedU16X2),
            "uint32" | "u32" => Some(Self::Uint32),
            "light_tile" | "lighttile" => Some(Self::LightTileU32),
            _ => None,
        }
    }

    pub fn byte_size(self) -> u32 {
        match self {
            Self::Uint32 => 4,
            Self::Float2 => 8,
            Self::Float3 => 12,
            Self::Float4 | Self::Joints | Self::Weights => 16,
            Self::PackedU20X3 => 8,
            Self::Unorm8X4 => 4,
            Self::PackedU16X2 | Self::LightTileU32 => 4,
        }
    }
}

/// Named mesh channel referenced from `#@vertex`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshVertexChannel {
    Position,
    Normal,
    Uv,
    AtlasRect,
    /// Optional per-vertex `float2` (`Mesh::lightmap_uvs` when present).
    ExtFloat2_0,
    /// Optional per-vertex `float4` (`Mesh::lightmap_styles` when present).
    ExtFloat4_0,
    BakedColor,
    LightTile,
    Joints,
    Weights,
}

impl MeshVertexChannel {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "position" | "pos" => Some(Self::Position),
            "normal" | "nrm" => Some(Self::Normal),
            "uv" | "texcoord" | "texcoord0" => Some(Self::Uv),
            "atlas_rect" | "atlas" => Some(Self::AtlasRect),
            "ext_float2_0" | "float2_0" | "user_float2_0" => Some(Self::ExtFloat2_0),
            "ext_float4_0" | "float4_0" | "user_float4_0" => Some(Self::ExtFloat4_0),
            "baked_color" | "color" => Some(Self::BakedColor),
            "light_tile" | "light_data" => Some(Self::LightTile),
            "joints" => Some(Self::Joints),
            "weights" => Some(Self::Weights),
            _ => None,
        }
    }

    pub fn default_type(self) -> VertexAttrType {
        match self {
            Self::Position => VertexAttrType::Float3,
            Self::Normal => VertexAttrType::Float3,
            Self::Uv | Self::ExtFloat2_0 => VertexAttrType::Float2,
            Self::AtlasRect | Self::ExtFloat4_0 | Self::Weights => VertexAttrType::Float4,
            Self::BakedColor => VertexAttrType::Unorm8X4,
            Self::LightTile => VertexAttrType::LightTileU32,
            Self::Joints => VertexAttrType::Joints,
        }
    }

    pub fn allows_type(self, ty: VertexAttrType) -> bool {
        ty == self.default_type()
            || matches!(
                (self, ty),
                (Self::Position, VertexAttrType::PackedU20X3)
                    | (Self::Uv, VertexAttrType::PackedU16X2)
                    | (Self::BakedColor, VertexAttrType::Unorm8X4)
                    | (Self::LightTile, VertexAttrType::LightTileU32)
            )
    }

    #[deprecated(note = "use default_type")]
    pub fn expected_type(self) -> VertexAttrType {
        self.default_type()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexAttributeSpec {
    pub channel: MeshVertexChannel,
    pub ty: VertexAttrType,
    pub location: u32,
    pub offset: u32,
}

/// Packed vertex stride + shader `@location` mapping.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayoutSpec {
    pub attrs: Vec<VertexAttributeSpec>,
    pub stride: u32,
}

impl VertexLayoutSpec {
    pub fn layout_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }

    pub fn is_skinned(&self) -> bool {
        self.attrs.iter().any(|a| {
            matches!(
                a.channel,
                MeshVertexChannel::Joints | MeshVertexChannel::Weights
            )
        })
    }

    /// `#@vertex position float3, normal float3, uv float2, ...`
    pub fn parse_vertex_line(rest: &str) -> Result<Self, String> {
        let tokens: Vec<&str> = rest
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.len() < 2 || !tokens.len().is_multiple_of(2) {
            return Err(format!("expected pairs name type, got: {rest}"));
        }
        let mut attrs = Vec::new();
        let mut offset = 0u32;
        for chunk in tokens.chunks(2) {
            let channel = MeshVertexChannel::parse(chunk[0])
                .ok_or_else(|| format!("unknown vertex channel '{}'", chunk[0]))?;
            let ty = VertexAttrType::parse(chunk[1])
                .ok_or_else(|| format!("unknown vertex type '{}'", chunk[1]))?;
            if !channel.allows_type(ty)
                && !matches!(
                    channel,
                    MeshVertexChannel::Joints | MeshVertexChannel::Weights
                )
            {
                return Err(format!(
                    "channel '{}' does not accept type {:?}",
                    chunk[0], ty
                ));
            }
            let location = attrs.len() as u32;
            attrs.push(VertexAttributeSpec {
                channel,
                ty,
                location,
                offset,
            });
            offset += ty.byte_size();
        }
        let stride = align_stride(offset);
        Ok(Self { attrs, stride })
    }

    /// Shaders without `#@vertex`: pos + normal + uv (32 bytes).
    pub fn legacy_basic() -> Self {
        Self::parse_vertex_line("position float3 normal float3 uv float2").expect("legacy_basic")
    }

    /// Historical extended mesh (80-byte stride, matches old `MeshVertex`).
    pub fn legacy_extended() -> Self {
        Self::parse_vertex_line(
            "position float3 normal float3 uv float2 atlas_rect float4 ext_float2_0 float2 ext_float4_0 float4",
        )
        .map(|mut s| {
            s.stride = 80;
            s
        })
        .expect("legacy_extended")
    }

    /// Default skinned studio mesh (64 bytes).
    pub fn legacy_skinned() -> Self {
        Self::parse_vertex_line(
            "position float3 normal float3 uv float2 joints uint4 weights float4",
        )
        .expect("legacy_skinned")
    }

    /// 20-byte quantized layout (Sodium-style packing).
    pub fn compact_chunk() -> Self {
        Self::parse_vertex_line(
            "position packed_u20x3, baked_color unorm8x4, texcoord packed_u16x2, light_tile light_tile",
        )
        .expect("compact_chunk")
    }

    /// Pick layout when `#@vertex` is omitted.
    pub fn legacy_from_shader_source(meta_skinned: bool, wgsl: &str, lm_surface: bool) -> Self {
        if meta_skinned {
            return Self::legacy_skinned();
        }
        if wgsl.contains("packed_u20x3") || wgsl.contains("baked_color") {
            return Self::compact_chunk();
        }
        if lm_surface
            || wgsl.contains("atlas_rect")
            || wgsl.contains("lm_uv")
            || wgsl.contains("lm_styles")
        {
            return Self::legacy_extended();
        }
        Self::legacy_basic()
    }
}

fn align_stride(end: u32) -> u32 {
    let mut stride = end;
    if stride % 4 != 0 {
        stride += 4 - (stride % 4);
    }
    stride
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_voxel_like_layout() {
        let l = VertexLayoutSpec::parse_vertex_line(
            "position float3, normal float3, uv float2, atlas_rect float4, ext_float2_0 float2, ext_float4_0 float4",
        )
        .unwrap();
        assert_eq!(l.attrs.len(), 6);
        assert_eq!(l.stride, 72);
    }

    #[test]
    fn parse_compact_layout() {
        let l = VertexLayoutSpec::compact_chunk();
        assert_eq!(l.stride, 20);
    }

    #[test]
    fn legacy_extended_stride_80() {
        assert_eq!(VertexLayoutSpec::legacy_extended().stride, 80);
    }
}
