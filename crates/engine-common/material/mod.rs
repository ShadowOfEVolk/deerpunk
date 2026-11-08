//! Material system — shader-driven material assets with inline metadata.
//!
//! See `.tasks/todo/TASK-001` through `TASK-009` for the full design.
//!
//! Layers:
//! - [`shader_meta`] — `#@` decorator parsing → `ShaderMeta`
//! - [`preprocessor`] — strip `#@`, apply `#define`, output clean WGSL
//! - [`asset`] — `MaterialAsset` (JSON/CBOR serializable)
//! - [`handle`] — `MaterialHandle`, `ShaderId`, `GpuMaterialIndex`
//! - [`component`] — `DrawMaterial` ECS component
//! - [`lighting`] — `SceneLighting` (scene-level, not per-material)

pub mod asset;
pub mod component;
pub mod handle;
pub mod lighting;
pub mod pipeline_desc;
pub mod preprocessor;
pub mod shader_meta;
pub mod vertex_layout;

pub use asset::{MaterialAsset, PropertyValueSer};
pub use component::DrawMaterial;
pub use handle::{
    GpuMaterialIndex, MaterialHandle, ShaderId, MATERIAL_HANDLE_NONE, SHADER_ID_NONE,
};
pub use lighting::SceneLighting;
pub use pipeline_desc::PipelineDesc;
pub use preprocessor::{screen_dither_preprocessor_defines, ShaderError, ShaderPreprocessor};
pub use shader_meta::{PropDefault, PropertyKind, RenderPass, ShaderMeta, ShaderPropertyMeta};
pub use vertex_layout::{MeshVertexChannel, VertexAttrType, VertexAttributeSpec, VertexLayoutSpec};
