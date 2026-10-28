//! Built-in [`crate::Loader`] implementations (host / engine).

pub mod audio_clip;
mod fbx;
mod file;
pub mod obj;
mod shader;
mod texgen;
mod transform;
mod wad;

pub use audio_clip::AudioClip;
pub use file::FileLoader;
pub use shader::{ShaderLoader, ShaderSource};
pub use texgen::{fallback_name_to_texgen_uri, TexgenLoader};
pub use wad::WadLoader;

use std::path::Path;

use crate::ResourceManager;

/// Register built-in host loaders (`file`, `wad`, `shader`, `texgen`). Game registers `bsp` etc. via [`ResourceManager::register_loader`].
pub fn register_default_loaders(rm: &ResourceManager, gamedir: &Path) {
    rm.set_base_dir(gamedir);
    rm.register_loader(Box::new(FileLoader {
        root: gamedir.to_path_buf(),
    }));
    rm.register_loader(Box::new(TexgenLoader));
    rm.register_loader(Box::new(ShaderLoader {
        root: gamedir.to_path_buf(),
    }));
    rm.register_loader(Box::new(WadLoader {
        root: gamedir.to_path_buf(),
    }));
}
