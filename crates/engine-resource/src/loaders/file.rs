//! `file://` — meshes (obj/fbx), images (png/jpg).

use std::path::PathBuf;

use crate::loader::{LoadedAsset, Loader};
use crate::resolve_content_path;
use crate::wad::RgbaImage;

use super::audio_clip::AudioClip;
use super::fbx;
use super::obj;
use super::transform;

pub struct FileLoader {
    pub root: PathBuf,
}

impl Loader for FileLoader {
    fn id(&self) -> &'static str {
        "file"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["obj", "fbx", "png", "jpg", "jpeg", "ogg", "wav"]
    }

    fn load(&self, path: &str, query: &str) -> Result<LoadedAsset, String> {
        let fs_path = resolve_content_path(&self.root, path);

        if path.ends_with(".ogg") || path.ends_with(".wav") {
            let data = std::fs::read(&fs_path)
                .map_err(|e| format!("cannot read {}: {e}", fs_path.display()))?;
            let ext = if path.ends_with(".wav") { "wav" } else { "ogg" };
            return Ok(LoadedAsset {
                data: Box::new(AudioClip {
                    bytes: data.clone(),
                    extension: ext.to_string(),
                }),
                size: data.len() as u64,
                type_name: "audio",
            });
        }

        if path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg") {
            let data = std::fs::read(&fs_path)
                .map_err(|e| format!("cannot read {}: {e}", fs_path.display()))?;
            let img = image::load_from_memory(&data)
                .map_err(|e| format!("decode image {path}: {e}"))?
                .into_rgba8();
            let (w, h) = img.dimensions();
            let rgba = RgbaImage {
                width: w,
                height: h,
                pixels: img.into_raw(),
            };
            return Ok(LoadedAsset {
                data: Box::new(rgba),
                size: data.len() as u64,
                type_name: "rgba",
            });
        }

        let size = std::fs::metadata(&fs_path).map(|m| m.len()).unwrap_or(0);
        let mut mesh = if path.ends_with(".obj") {
            let data = std::fs::read_to_string(&fs_path)
                .map_err(|e| format!("cannot read {}: {e}", fs_path.display()))?;
            obj::parse_obj(&data)?
        } else if path.ends_with(".fbx") {
            let data = std::fs::read(&fs_path)
                .map_err(|e| format!("cannot read {}: {e}", fs_path.display()))?;
            fbx::parse_fbx(&data)?
        } else {
            return Err(format!("unrecognised file extension: {path}"));
        };
        let transform = transform::parse_mesh_transform(query);
        transform::apply_mesh_transform(&mut mesh, &transform);
        Ok(LoadedAsset {
            data: Box::new(mesh),
            size,
            type_name: "mesh",
        })
    }
}
