//! GoldSrc BSP30 lightmap extents, per-style layers, atlas pack (4 array layers).

use crate::wad::RgbaImage;

/// Luxels per world texcoord unit (Quake/HL).
pub const LM_SCALE: i32 = 16;
/// Max style slots per face (BSP Face.styles).
pub const LM_STYLE_SLOTS: usize = 4;

#[derive(Clone)]
pub struct FaceLightmap {
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub width: u32,
    pub height: u32,
    pub texturemins: [i32; 2],
    /// Style indices (255 = unused slot).
    pub styles: [u8; 4],
    /// RGB per style slot (`width*height*3`), `None` if unused.
    pub layers_rgb: [Option<Vec<u8>>; LM_STYLE_SLOTS],
}

/// Four atlas layers (same W×H), sampled as `texture_2d_array`.
#[derive(Clone)]
pub struct LightmapAtlasLayers {
    pub width: u32,
    pub height: u32,
    pub layers: [RgbaImage; LM_STYLE_SLOTS],
}

pub fn calc_extents(s_coords: &[f32], t_coords: &[f32]) -> Option<([i32; 2], [i32; 2], u32, u32)> {
    if s_coords.is_empty() || t_coords.is_empty() {
        return None;
    }
    let mut mins = [f32::MAX, f32::MAX];
    let mut maxs = [f32::MIN, f32::MIN];
    for (&s, &t) in s_coords.iter().zip(t_coords.iter()) {
        mins[0] = mins[0].min(s);
        maxs[0] = maxs[0].max(s);
        mins[1] = mins[1].min(t);
        maxs[1] = maxs[1].max(t);
    }
    let bmins = [
        (mins[0] / LM_SCALE as f32).floor() as i32,
        (mins[1] / LM_SCALE as f32).floor() as i32,
    ];
    let bmaxs = [
        (maxs[0] / LM_SCALE as f32).ceil() as i32,
        (maxs[1] / LM_SCALE as f32).ceil() as i32,
    ];
    let texturemins = [bmins[0] * LM_SCALE, bmins[1] * LM_SCALE];
    let w = (bmaxs[0] - bmins[0] + 1) as u32;
    let h = (bmaxs[1] - bmins[1] + 1) as u32;
    if w == 0 || h == 0 || w > 256 || h > 256 {
        return None;
    }
    Some((texturemins, [bmaxs[0] - bmins[0], bmaxs[1] - bmins[1]], w, h))
}

/// Extract up to 4 style layers (not summed).
pub fn extract_style_layers(
    lighting: &[u8],
    lightofs: i32,
    styles: [u8; 4],
    w: u32,
    h: u32,
) -> Option<FaceLightmap> {
    if lightofs < 0 {
        return None;
    }
    let luxels = (w as usize).checked_mul(h as usize)?;
    let block = luxels.checked_mul(3)?;
    let mut ofs = lightofs as usize;
    let mut layers_rgb: [Option<Vec<u8>>; LM_STYLE_SLOTS] = [None, None, None, None];
    let mut any = false;
    for (slot, &style) in styles.iter().enumerate() {
        if style == 255 {
            break;
        }
        if ofs.checked_add(block).is_none_or(|end| end > lighting.len()) {
            break;
        }
        layers_rgb[slot] = Some(lighting[ofs..ofs + block].to_vec());
        ofs += block;
        any = true;
    }
    if !any {
        return None;
    }
    Some(FaceLightmap {
        atlas_x: 0,
        atlas_y: 0,
        width: w,
        height: h,
        texturemins: [0, 0],
        styles,
        layers_rgb,
    })
}

struct ShelfPacker {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    shelf_h: u32,
}

impl ShelfPacker {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            x: 0,
            y: 0,
            shelf_h: 0,
        }
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let bw = w + 2;
        let bh = h + 2;
        if bw > self.width || bh > self.height {
            return None;
        }
        if self.x + bw > self.width {
            self.y += self.shelf_h;
            self.x = 0;
            self.shelf_h = 0;
        }
        if self.y + bh > self.height {
            return None;
        }
        let ix = self.x + 1;
        let iy = self.y + 1;
        self.x += bw;
        self.shelf_h = self.shelf_h.max(bh);
        Some((ix, iy))
    }
}

/// Pack faces into a 4-layer RGBA atlas (same UV for all style slots).
pub fn pack_atlas_layers(faces: &mut [Option<FaceLightmap>]) -> Option<LightmapAtlasLayers> {
    let mut area = 0u64;
    let mut max_side = 1u32;
    for f in faces.iter().flatten() {
        area += (f.width as u64 + 2) * (f.height as u64 + 2);
        max_side = max_side.max(f.width + 2).max(f.height + 2);
    }
    if area == 0 {
        return None;
    }
    let mut size = 512u32.max(max_side.next_power_of_two());
    while (size as u64) * (size as u64) < area * 2 && size < 4096 {
        size *= 2;
    }

    loop {
        let mut packer = ShelfPacker::new(size, size);
        let mut ok = true;
        let mut placed: Vec<(usize, u32, u32)> = Vec::new();
        for (i, slot) in faces.iter().enumerate() {
            let Some(f) = slot else { continue };
            match packer.alloc(f.width, f.height) {
                Some((x, y)) => placed.push((i, x, y)),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            if size >= 4096 {
                return None;
            }
            size *= 2;
            continue;
        }

        // White clear: samples landing outside any face patch must stay neutral.
        let mut layer_pixels: [Vec<u8>; LM_STYLE_SLOTS] =
            std::array::from_fn(|_| {
                let mut v = Vec::with_capacity((size * size * 4) as usize);
                for _ in 0..(size * size) {
                    v.extend_from_slice(&[255, 255, 255, 255]);
                }
                v
            });

        for (i, x, y) in placed {
            if let Some(f) = faces[i].as_mut() {
                f.atlas_x = x;
                f.atlas_y = y;
                for slot in 0..LM_STYLE_SLOTS {
                    blit_face_layer(&mut layer_pixels[slot], size, f, slot);
                }
            }
        }

        let layers = std::array::from_fn(|i| RgbaImage {
            width: size,
            height: size,
            pixels: std::mem::take(&mut layer_pixels[i]),
        });
        return Some(LightmapAtlasLayers {
            width: size,
            height: size,
            layers,
        });
    }
}

fn blit_face_layer(atlas: &mut [u8], atlas_w: u32, face: &FaceLightmap, slot: usize) {
    let w = face.width;
    let h = face.height;
    let ax = face.atlas_x;
    let ay = face.atlas_y;
    // Unused style slot: fill white (not black). Accidental samples then don't
    // create black speckles that look like z-fighting when LM is on.
    let Some(rgb) = face.layers_rgb[slot].as_ref() else {
        for ly in 0..h {
            for lx in 0..w {
                put(atlas, atlas_w, ax + lx, ay + ly, 255, 255, 255);
            }
        }
        return;
    };
    for ly in 0..h {
        for lx in 0..w {
            let src = ((ly * w + lx) * 3) as usize;
            put(
                atlas,
                atlas_w,
                ax + lx,
                ay + ly,
                rgb[src],
                rgb[src + 1],
                rgb[src + 2],
            );
        }
    }
    for lx in 0..w {
        let top = sample_rgb(rgb, w, lx, 0);
        let bot = sample_rgb(rgb, w, lx, h - 1);
        put(atlas, atlas_w, ax + lx, ay.wrapping_sub(1), top.0, top.1, top.2);
        put(atlas, atlas_w, ax + lx, ay + h, bot.0, bot.1, bot.2);
    }
    for ly in 0..h {
        let left = sample_rgb(rgb, w, 0, ly);
        let right = sample_rgb(rgb, w, w - 1, ly);
        put(atlas, atlas_w, ax.wrapping_sub(1), ay + ly, left.0, left.1, left.2);
        put(atlas, atlas_w, ax + w, ay + ly, right.0, right.1, right.2);
    }
    let c00 = sample_rgb(rgb, w, 0, 0);
    let c10 = sample_rgb(rgb, w, w - 1, 0);
    let c01 = sample_rgb(rgb, w, 0, h - 1);
    let c11 = sample_rgb(rgb, w, w - 1, h - 1);
    put(atlas, atlas_w, ax.wrapping_sub(1), ay.wrapping_sub(1), c00.0, c00.1, c00.2);
    put(atlas, atlas_w, ax + w, ay.wrapping_sub(1), c10.0, c10.1, c10.2);
    put(atlas, atlas_w, ax.wrapping_sub(1), ay + h, c01.0, c01.1, c01.2);
    put(atlas, atlas_w, ax + w, ay + h, c11.0, c11.1, c11.2);
}

fn sample_rgb(rgb: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * w + x) * 3) as usize;
    (rgb[i], rgb[i + 1], rgb[i + 2])
}

fn put(atlas: &mut [u8], atlas_w: u32, x: u32, y: u32, r: u8, g: u8, b: u8) {
    if x >= atlas_w {
        return;
    }
    let i = ((y * atlas_w + x) * 4) as usize;
    if i + 3 >= atlas.len() {
        return;
    }
    atlas[i] = r;
    atlas[i + 1] = g;
    atlas[i + 2] = b;
    atlas[i + 3] = 255;
}

pub fn lm_uv(face: &FaceLightmap, atlas_w: u32, atlas_h: u32, s: f32, t: f32) -> [f32; 2] {
    // Quake/HL: luxel center = (texcoord - texturemins)/16 + 0.5
    let mut u = (s - face.texturemins[0] as f32) / LM_SCALE as f32 + 0.5;
    let mut v = (t - face.texturemins[1] as f32) / LM_SCALE as f32 + 0.5;
    // Keep sample inside the face rect (+ padding is outside [0.5 .. w-0.5]).
    let umax = (face.width.max(1) as f32) - 0.5;
    let vmax = (face.height.max(1) as f32) - 0.5;
    u = u.clamp(0.5, umax.max(0.5));
    v = v.clamp(0.5, vmax.max(0.5));
    [
        (face.atlas_x as f32 + u) / atlas_w.max(1) as f32,
        (face.atlas_y as f32 + v) / atlas_h.max(1) as f32,
    ]
}

/// Style indices as f32 for vertex attribute (255 = unused).
pub fn styles_as_f32(styles: [u8; 4]) -> [f32; 4] {
    [
        styles[0] as f32,
        styles[1] as f32,
        styles[2] as f32,
        styles[3] as f32,
    ]
}
