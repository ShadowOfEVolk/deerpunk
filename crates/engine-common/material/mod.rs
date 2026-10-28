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

pub mod shader_meta;
pub mod preprocessor;
pub mod asset;
pub mod handle;
pub mod component;
pub mod lighting;
pub mod pipeline_desc;

pub use shader_meta::{
    PropDefault, PropertyKind, RenderPass, ShaderMeta, ShaderPropertyMeta,
};
pub use preprocessor::{ShaderPreprocessor, ShaderError};
pub use asset::{MaterialAsset, PropertyValueSer};
pub use handle::{GpuMaterialIndex, MaterialHandle, ShaderId, SHADER_ID_NONE, MATERIAL_HANDLE_NONE};
pub use component::DrawMaterial;
pub use lighting::SceneLighting;
pub use pipeline_desc::PipelineDesc;
