//! Size buckets for GoldSrc textures → `texture_2d_array` layers.

use crate::wad::RgbaImage;

/// Power-of-two buckets we support (square layers).
pub const BUCKET_SIZES: [u32; 4] = [64, 128, 256, 512];

pub fn pick_bucket(w: u32, h: u32) -> u32 {
    let edge = w.max(h).max(1);
    for &b in &BUCKET_SIZES {
        if edge <= b {
            return b;
        }
    }
    *BUCKET_SIZES.last().unwrap()
}

/// Nearest-neighbor scale (or letterbox pad) into a square `size×size` layer.
pub fn resize_to_bucket(src: &RgbaImage, size: u32) -> RgbaImage {
    let size = size.max(1);
    if src.width == size && src.height == size {
        return src.clone();
    }
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let sx = (x as u64 * src.width as u64 / size as u64) as u32;
            let sy = (y as u64 * src.height as u64 / size as u64) as u32;
            let sx = sx.min(src.width.saturating_sub(1));
            let sy = sy.min(src.height.saturating_sub(1));
            let si = ((sy * src.width + sx) * 4) as usize;
            let di = ((y * size + x) * 4) as usize;
            pixels[di..di + 4].copy_from_slice(&src.pixels[si..si + 4]);
        }
    }
    RgbaImage {
        width: size,
        height: size,
        pixels,
    }
}
