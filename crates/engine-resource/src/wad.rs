//! GoldSrc WAD3 + embedded BSP miptex → RGBA8.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// 1×1 solid color (cached well via `texgen://` presets).
    pub fn solid(rgba: [u8; 4]) -> Self {
        Self {
            width: 1,
            height: 1,
            pixels: rgba.to_vec(),
        }
    }

    pub fn magenta(w: u32, h: u32) -> Self {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            pixels[i * 4] = 255;
            pixels[i * 4 + 1] = 0;
            pixels[i * 4 + 2] = 255;
            pixels[i * 4 + 3] = 255;
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }

    pub fn checker(w: u32, h: u32) -> Self {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let c = if ((x / 8) ^ (y / 8)) & 1 == 0 {
                    200
                } else {
                    40
                };
                let i = ((y * w + x) * 4) as usize;
                pixels[i] = c;
                pixels[i + 1] = 40;
                pixels[i + 2] = c;
                pixels[i + 3] = 255;
            }
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }
}

/// In-memory WAD3 texture directory (name → RGBA mip0).
pub struct WadArchive {
    textures: HashMap<String, RgbaImage>,
}

impl WadArchive {
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(&data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() < 12 {
            return Err("WAD too small".into());
        }
        let magic = &data[0..4];
        if magic != b"WAD2" && magic != b"WAD3" {
            return Err(format!("bad WAD magic {:?}", magic));
        }
        let numlumps = i32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let infotableofs = i32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        let mut textures = HashMap::new();
        for i in 0..numlumps {
            let off = infotableofs + i * 32;
            if off + 32 > data.len() {
                break;
            }
            let filepos = i32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
            let disksize = i32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap()) as usize;
            let typ = data[off + 12];
            // 0x43 = miptex ('C'), 0x42 = qpic — we only want miptex
            if typ != 0x43 && typ != b'C' {
                continue;
            }
            let name = cstr16(&data[off + 16..off + 32]);
            if name.is_empty() {
                continue;
            }
            let end = filepos.saturating_add(disksize).min(data.len());
            if filepos >= end {
                continue;
            }
            match decode_miptex(&data[filepos..end]) {
                Ok(img) => {
                    textures.insert(normalize_tex_name(&name), img);
                }
                Err(e) => log::debug!("[wad] skip '{name}': {e}"),
            }
        }
        Ok(Self { textures })
    }

    pub fn get(&self, name: &str) -> Option<&RgbaImage> {
        self.textures.get(&normalize_tex_name(name))
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }
}

/// Search gamedir (and optional explicit wad list from worldspawn) for textures.
pub struct WadSet {
    archives: Vec<WadArchive>,
}

impl WadSet {
    pub fn from_gamedir(gamedir: &Path, wad_names: &[String]) -> Self {
        let mut paths: Vec<PathBuf> = Vec::new();
        for w in wad_names {
            let w = w.trim().trim_end_matches(';');
            if w.is_empty() {
                continue;
            }
            let name = Path::new(w)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| w.to_string());
            let p = gamedir.join(&name);
            if p.is_file() {
                paths.push(p);
            }
        }
        // Always try common gamedir wads.
        for name in ["darkdemo.wad", "gfx.wad", "decals.wad", "halflife.wad"] {
            let p = gamedir.join(name);
            if p.is_file() && !paths.iter().any(|x| x == &p) {
                paths.push(p);
            }
        }
        if let Ok(rd) = std::fs::read_dir(gamedir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("wad")) == Some(true)
                    && !paths.iter().any(|x| x == &p)
                {
                    paths.push(p);
                }
            }
        }
        let mut archives = Vec::new();
        for p in &paths {
            match WadArchive::load_file(p) {
                Ok(a) => {
                    log::info!("[wad] loaded {} ({} textures)", p.display(), a.len());
                    archives.push(a);
                }
                Err(e) => log::warn!("[wad] {e}"),
            }
        }
        Self { archives }
    }

    pub fn get(&self, name: &str) -> Option<&RgbaImage> {
        for a in &self.archives {
            if let Some(img) = a.get(name) {
                return Some(img);
            }
        }
        None
    }
}

pub fn normalize_tex_name(name: &str) -> String {
    name.split('\0').next().unwrap_or(name).trim().to_ascii_lowercase()
}

fn cstr16(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Decode a HL miptex blob (embedded in BSP or WAD lump) to RGBA8 mip0.
pub fn decode_miptex(data: &[u8]) -> Result<RgbaImage, String> {
    if data.len() < 40 {
        return Err("miptex header too small".into());
    }
    let name = cstr16(&data[0..16]);
    let width = u32::from_le_bytes(data[16..20].try_into().unwrap());
    let height = u32::from_le_bytes(data[20..24].try_into().unwrap());
    let offset0 = u32::from_le_bytes(data[24..28].try_into().unwrap()) as usize;
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err(format!("bad size {width}x{height}"));
    }
    if offset0 == 0 {
        return Err("external miptex (no pixels)".into());
    }
    let mip0_len = (width * height) as usize;
    let mip0_end = offset0
        .checked_add(mip0_len)
        .ok_or("mip0 overflow")?;
    if mip0_end > data.len() {
        return Err("mip0 out of range".into());
    }
    let indices = &data[offset0..mip0_end];

    // Palette sits after all 4 mips.
    let mut pal_off = offset0;
    let mut w = width;
    let mut h = height;
    for _ in 0..4 {
        pal_off += (w * h) as usize;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    if pal_off + 2 + 768 > data.len() {
        // Some WADs store only 256*3 without count — try that.
        if pal_off + 768 <= data.len() {
            return indices_to_rgba(indices, width, height, &data[pal_off..pal_off + 768], &name);
        }
        return Err("palette missing".into());
    }
    let ncolors = u16::from_le_bytes(data[pal_off..pal_off + 2].try_into().unwrap()) as usize;
    let pal_bytes = ncolors.saturating_mul(3).max(768).min(data.len() - (pal_off + 2));
    let palette = &data[pal_off + 2..pal_off + 2 + pal_bytes];
    indices_to_rgba(indices, width, height, palette, &name)
}

fn indices_to_rgba(
    indices: &[u8],
    width: u32,
    height: u32,
    palette: &[u8],
    name: &str,
) -> Result<RgbaImage, String> {
    let chromakey = name.starts_with('{');
    let mut pixels = vec![0u8; indices.len() * 4];
    for (i, &idx) in indices.iter().enumerate() {
        let pi = idx as usize * 3;
        let (r, g, b, a) = if chromakey && idx == 255 {
            (0, 0, 0, 0)
        } else if pi + 2 < palette.len() {
            (palette[pi], palette[pi + 1], palette[pi + 2], 255)
        } else {
            (255, 0, 255, 255)
        };
        pixels[i * 4] = r;
        pixels[i * 4 + 1] = g;
        pixels[i * 4 + 2] = b;
        pixels[i * 4 + 3] = a;
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

/// Parse `"wad" "a.wad;b.wad;"` from BSP entities lump.
pub fn parse_wad_list_from_entities(entities: &str) -> Vec<String> {
    for block in entities.split('{') {
        if !block.to_ascii_lowercase().contains("worldspawn") {
            continue;
        }
        // crude key scan
        let mut keys = block.split('"').filter(|s| !s.trim().is_empty());
        while let Some(k) = keys.next() {
            let k = k.trim();
            if k.eq_ignore_ascii_case("wad") {
                if let Some(v) = keys.next() {
                    return v
                        .split([';', ':'])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }
    Vec::new()
}
