//! `internal://` — WGSL and other text assets compiled into the engine.

use crate::internal_assets::{internal_asset, normalize_internal_path};
use crate::loader::{LoadedAsset, Loader};
use crate::loaders::ShaderSource;

pub struct InternalLoader;

impl Loader for InternalLoader {
    fn id(&self) -> &'static str {
        "internal"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["wgsl"]
    }

    fn load(&self, path: &str, _query: &str) -> Result<LoadedAsset, String> {
        let key = normalize_internal_path(path);
        let wgsl = internal_asset(&key)
            .ok_or_else(|| format!("internal asset not found: internal://{key}"))?;
        let size = wgsl.len() as u64;
        Ok(LoadedAsset {
            data: Box::new(ShaderSource {
                wgsl: wgsl.to_string(),
            }),
            size,
            type_name: "shader",
        })
    }
}
