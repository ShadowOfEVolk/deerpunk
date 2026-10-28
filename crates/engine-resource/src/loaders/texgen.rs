//! Procedural RGBA textures: `texgen://white`, `texgen://normal`, …

use crate::loader::{LoadedAsset, Loader};
use crate::wad::RgbaImage;

pub struct TexgenLoader;

impl Loader for TexgenLoader {
    fn id(&self) -> &'static str {
        "texgen"
    }

    fn load(&self, path: &str, _query: &str) -> Result<LoadedAsset, String> {
        let img = match path {
            "white" => RgbaImage::solid([255, 255, 255, 255]),
            "black" => RgbaImage::solid([0, 0, 0, 255]),
            "normal" => RgbaImage::solid([128, 128, 255, 255]),
            "orm" => RgbaImage::solid([255, 255, 255, 255]),
            other => return Err(format!("unknown texgen preset: {other}")),
        };
        Ok(LoadedAsset {
            data: Box::new(img),
            size: 4,
            type_name: "rgba",
        })
    }
}

/// Map shader/material fallback names to explicit `texgen://` URIs.
pub fn fallback_name_to_texgen_uri(name: &str) -> Option<&'static str> {
    match name {
        "white" => Some("texgen://white"),
        "black" => Some("texgen://black"),
        "normal" => Some("texgen://normal"),
        "orm" => Some("texgen://orm"),
        _ => None,
    }
}
