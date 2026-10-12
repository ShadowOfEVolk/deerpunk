//! `wad://file.wad/texname` → [`crate::wad::RgbaImage`].

use std::path::Path;

use super::loader::{LoadedAsset, Loader};
use crate::wad::{self, RgbaImage, WadArchive};

pub struct WadLoader {
    /// Base directory for relative wad paths (usually gamedir).
    pub root: std::path::PathBuf,
}

impl Loader for WadLoader {
    fn id(&self) -> &'static str {
        "wad"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["wad"]
    }

    fn load(&self, path: &str, _query: &str) -> Result<LoadedAsset, String> {
        // path: "darkdemo.wad/concrete" or bare file.wad (whole archive — not yet)
        let (wad_file, tex) = match path.split_once('/') {
            Some((w, t)) => (Some(w), t),
            None => {
                // file://darkdemo.wad → load archive listing not supported; need /tex
                if path.ends_with(".wad") {
                    return Err("wad file load needs texture: file.wad/texname or wad://file.wad/texname".into());
                }
                (None, path)
            }
        };
        let tex = wad::normalize_tex_name(tex);
        if let Some(w) = wad_file {
            let p = if Path::new(w).is_absolute() {
                Path::new(w).to_path_buf()
            } else {
                self.root.join(w)
            };
            let arch = WadArchive::load_file(&p)?;
            let img = arch
                .get(&tex)
                .cloned()
                .ok_or_else(|| format!("texture '{tex}' not in {}", p.display()))?;
            let size = img.pixels.len() as u64;
            return Ok(LoadedAsset {
                data: Box::new(img),
                size,
                type_name: "rgba",
            });
        }
        Err("wad:// requires file.wad/texname".into())
    }
}

impl WadLoader {
    pub fn load_rgba(root: &Path, uri_path: &str) -> Result<RgbaImage, String> {
        let loader = WadLoader {
            root: root.to_path_buf(),
        };
        let loaded = loader.load(uri_path, "")?;
        loaded
            .data
            .downcast::<RgbaImage>()
            .map(|b| *b)
            .map_err(|_| "type mismatch".into())
    }
}
