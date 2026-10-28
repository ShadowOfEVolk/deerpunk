//! `shader://` and `file://…/*.wgsl` → [`ShaderSource`] (raw WGSL text).

use std::path::{Path, PathBuf};

use crate::loader::{LoadedAsset, Loader};
use crate::resolve_content_path;

/// CPU-side shader asset (preprocess/compile happens in client [`ShaderRegistry`]).
#[derive(Clone, Debug)]
pub struct ShaderSource {
    pub wgsl: String,
}

pub struct ShaderLoader {
    /// Gamedir (or `SHADER_DIR` for engine bootstrap).
    pub root: PathBuf,
}

impl Loader for ShaderLoader {
    fn id(&self) -> &'static str {
        "shader"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["wgsl"]
    }

    fn load(&self, path: &str, _query: &str) -> Result<LoadedAsset, String> {
        let fs_path = resolve_content_path(&self.root, path);
        let wgsl = std::fs::read_to_string(&fs_path)
            .map_err(|e| format!("cannot read {}: {e}", fs_path.display()))?;
        let size = wgsl.len() as u64;
        Ok(LoadedAsset {
            data: Box::new(ShaderSource { wgsl }),
            size,
            type_name: "shader",
        })
    }
}
