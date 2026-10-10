//! `file://…/map.bsp` → [`crate::bsp30::BspWorldMesh`].

use std::path::{Path, PathBuf};

use super::loader::{LoadedAsset, Loader};
use crate::bsp30;

/// BSP30 map loader. Uses [`ResourceManager`](super::ResourceManager) base dir as gamedir for WADs.
pub struct BspLoader {
    pub gamedir: PathBuf,
}

impl BspLoader {
    pub fn new(gamedir: impl Into<PathBuf>) -> Self {
        Self {
            gamedir: gamedir.into(),
        }
    }

    pub fn set_gamedir(&mut self, gamedir: impl Into<PathBuf>) {
        self.gamedir = gamedir.into();
    }
}

impl Loader for BspLoader {
    fn id(&self) -> &'static str {
        "bsp"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["bsp"]
    }

    fn load(&self, path: &str, _query: &str) -> Result<LoadedAsset, String> {
        let p = Path::new(path);
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.gamedir.join(p)
        };
        let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
        let world = bsp30::load_world_mesh(&full, &self.gamedir)?;
        Ok(LoadedAsset {
            data: Box::new(world),
            size,
            type_name: "bsp",
        })
    }
}
