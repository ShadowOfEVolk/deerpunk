//! Shader metadata extracted from `#@` decorator lines in `.wgsl` files.
//!
//! Format (inline in WGSL, stripped by preprocessor before naga):
//! ```wgsl
//! #@shader pass=mesh_opaque queue=1000
//!
//! #@prop _BaseColor color [1,1,1,1]
//! @group(0) @binding(0) var<uniform> u_material: MaterialUniform;
//!
//! #@prop _BaseMap texture2d white
//! @group(0) @binding(1) var base_map: texture_2d<f32>;
//! ```
//!
//! Binding index comes from the actual `@binding(N)` in WGSL — not duplicated
//! in the `#@prop` line. The preprocessor associates each `#@prop` with the
//! next `@group/@binding var` or struct field below it.

/// Render pass / pipeline category. Parsed from `#@shader pass=...`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenderPass {
    /// Opaque PBR mesh, depth-tested, forward.
    MeshOpaque,
    /// Alpha-blended mesh, sorted back-to-front.
    MeshTransparent,
    /// Fullscreen skybox, drawn before opaque, no depth write.
    Skybox,
    /// Fullscreen post effect (tonemap, blur, etc.).
    Fullscreen,
    /// Terrain splat (existing path, may converge later).
    Terrain,
}

impl RenderPass {
    /// Parse from string (used by `#@shader pass=...`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "mesh_opaque" => Some(Self::MeshOpaque),
            "mesh_transparent" => Some(Self::MeshTransparent),
            "skybox" => Some(Self::Skybox),
            "fullscreen" => Some(Self::Fullscreen),
            "terrain" => Some(Self::Terrain),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MeshOpaque => "mesh_opaque",
            Self::MeshTransparent => "mesh_transparent",
            Self::Skybox => "skybox",
            Self::Fullscreen => "fullscreen",
            Self::Terrain => "terrain",
        }
    }
}

/// Property kind — determines how a value is stored, validated, and bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    Float,
    Color,
    Vec3,
    Vec4,
    Texture2d,
    TextureCube,
}

impl PropertyKind {
    /// Parse from string (used by `#@prop name kind default`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "float" => Some(Self::Float),
            "color" => Some(Self::Color),
            "vec3" => Some(Self::Vec3),
            "vec4" => Some(Self::Vec4),
            "texture2d" => Some(Self::Texture2d),
            "texture_cube" | "texturecube" => Some(Self::TextureCube),
            _ => None,
        }
    }
}

/// A single property's metadata, extracted from a `#@prop` line.
#[derive(Clone, Debug)]
pub struct ShaderPropertyMeta {
    /// Property name, e.g. `"_BaseColor"`. Must match keys in `MaterialAsset`.
    pub name: String,
    /// Value kind (float, color, texture2d, ...).
    pub kind: PropertyKind,
    /// Default value when the material does not specify this property.
    pub default: PropDefault,
    /// Binding index, filled from the `@binding(N)` of the associated WGSL entry.
    /// None until the preprocessor associates it with a binding.
    pub binding: Option<u32>,
}

/// Default value for a property, parsed from the `#@prop` line.
#[derive(Clone, Debug)]
pub enum PropDefault {
    Float(f32),
    Color([f32; 4]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    /// Texture fallback name: "white", "black", "normal", etc.
    Texture(String),
}

/// Shader-level metadata, extracted from `#@shader` line(s).
#[derive(Clone, Debug, Default)]
pub struct ShaderMeta {
    /// Render pass (pipeline category). None = error / not set.
    pub pass: Option<RenderPass>,
    /// Sort key within the pass (lower = drawn first).
    pub queue: u32,
    /// Skinned mesh opaque: group 0 includes a bone palette (`mesh_skinned` layout).
    pub skinned: bool,
    /// Property declarations, in source order.
    pub properties: Vec<ShaderPropertyMeta>,
}
