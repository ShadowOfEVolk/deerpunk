//! Simple shelf atlas packer for GoldSrc albedo textures.

use crate::wad::RgbaImage;

#[derive(Clone, Copy, Debug)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub struct Atlas {
    pub image: RgbaImage,
    pub rects: Vec<AtlasRect>,
}

/// Pack images into one RGBA atlas. `pad` pixels of border (duplicated edge) per side.
pub fn pack(images: &[&RgbaImage], pad: u32) -> Atlas {
    if images.is_empty() {
        return Atlas {
            image: RgbaImage::magenta(16, 16),
            rects: Vec::new(),
        };
    }

    let mut order: Vec<usize> = (0..images.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(images[i].height.max(images[i].width)));

    let mut placements: Vec<Option<AtlasRect>> = vec![None; images.len()];
    let mut shelves: Vec<(u32, u32, u32)> = Vec::new(); // y, height, x_cursor
    let mut atlas_w = 0u32;
    let mut atlas_h = 0u32;

    const MAX_W: u32 = 4096;

    for &idx in &order {
        let img = images[idx];
        let bw = img.width + pad * 2;
        let bh = img.height + pad * 2;

        let mut placed = None;
        for shelf in &mut shelves {
            let (sy, sh, sx) = *shelf;
            if bh <= sh && sx + bw <= MAX_W {
                placed = Some(AtlasRect {
                    x: sx + pad,
                    y: sy + pad,
                    w: img.width,
                    h: img.height,
                });
                shelf.2 = sx + bw;
                atlas_w = atlas_w.max(shelf.2);
                break;
            }
        }
        if placed.is_none() {
            let y = atlas_h;
            placed = Some(AtlasRect {
                x: pad,
                y: y + pad,
                w: img.width,
                h: img.height,
            });
            shelves.push((y, bh, bw));
            atlas_h = y + bh;
            atlas_w = atlas_w.max(bw);
        }
        placements[idx] = placed;
    }

    // Round up to multiple of 4 for wgpu bytes_per_row comfort.
    atlas_w = atlas_w.max(1).div_ceil(4) * 4;
    atlas_h = atlas_h.max(1);

    let mut pixels = vec![0u8; (atlas_w * atlas_h * 4) as usize];
    let rects: Vec<AtlasRect> = placements.into_iter().map(|r| r.unwrap()).collect();

    for (img, rect) in images.iter().zip(rects.iter()) {
        blit(img, &mut pixels, atlas_w, rect, pad);
    }

    Atlas {
        image: RgbaImage {
            width: atlas_w,
            height: atlas_h,
            pixels,
        },
        rects,
    }
}

fn blit(src: &RgbaImage, dst: &mut [u8], dst_w: u32, rect: &AtlasRect, pad: u32) {
    let x0 = rect.x.saturating_sub(pad);
    let y0 = rect.y.saturating_sub(pad);
    let x1 = (rect.x + rect.w + pad).min(dst_w);
    let y1 = (rect.y + rect.h + pad).min((dst.len() as u32 / 4) / dst_w.max(1));

    for y in y0..y1 {
        for x in x0..x1 {
            let sx = if x < rect.x {
                0
            } else if x >= rect.x + rect.w {
                src.width.saturating_sub(1)
            } else {
                x.saturating_sub(rect.x).min(src.width.saturating_sub(1))
            };
            let sy = if y < rect.y {
                0
            } else if y >= rect.y + rect.h {
                src.height.saturating_sub(1)
            } else {
                y.saturating_sub(rect.y).min(src.height.saturating_sub(1))
            };
            let si = ((sy * src.width + sx) * 4) as usize;
            let di = ((y * dst_w + x) * 4) as usize;
            if si + 3 < src.pixels.len() && di + 3 < dst.len() {
                dst[di..di + 4].copy_from_slice(&src.pixels[si..si + 4]);
            }
        }
    }
}

/// Normalized atlas rect for shader wrap: `[min_u, min_v, size_u, size_v]`.
/// Half-texel inset reduces bleeding into padding.
pub fn rect_norm(rect: &AtlasRect, atlas_w: u32, atlas_h: u32) -> [f32; 4] {
    let aw = atlas_w.max(1) as f32;
    let ah = atlas_h.max(1) as f32;
    let inset = 0.5;
    let tw = rect.w.max(1) as f32;
    let th = rect.h.max(1) as f32;
    let w = (tw - inset * 2.0).max(1.0);
    let h = (th - inset * 2.0).max(1.0);
    [
        (rect.x as f32 + inset) / aw,
        (rect.y as f32 + inset) / ah,
        w / aw,
        h / ah,
    ]
}
