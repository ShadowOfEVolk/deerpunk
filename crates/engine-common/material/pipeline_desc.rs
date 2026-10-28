//! Serializable render pipeline identity (CPU). GPU pipelines are built on the client from
//! [`ShaderRegistry`] + [`MaterialAsset`] properties.

use serde::{Deserialize, Serialize};

/// References a shader asset and variant; pairs with material property JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineDesc {
    pub shader_uri: String,
    #[serde(default)]
    pub shader_defines: Vec<String>,
}

impl PipelineDesc {
    pub fn from_material(asset: &super::asset::MaterialAsset) -> Self {
        Self {
            shader_uri: asset.shader.clone(),
            shader_defines: asset.shader_defines.clone(),
        }
    }
}
