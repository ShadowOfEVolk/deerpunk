//! Compact chunk-style vertex encoding (matches `#@vertex` compact layout, 20-byte stride).

/// Stride for `VertexLayoutSpec::compact_chunk()`.
pub const COMPACT_CHUNK_VERTEX_STRIDE: u32 = 20;

const POSITION_BITS: u32 = 20;
const POSITION_MAX: u32 = (1 << POSITION_BITS) - 1;

pub fn encode_position_u20x3(p: [f32; 3]) -> [u32; 2] {
    let mut word0 = 0u32;
    let mut word1 = 0u32;
    for (axis, coord) in p.iter().enumerate() {
        let norm = ((coord + 8.0) / 32.0).clamp(0.0, 1.0);
        let q = (norm * POSITION_MAX as f32).round() as u32;
        let lo = q & 0x3FF;
        let hi = (q >> 10) & 0x3FF;
        let shift = (axis as u32) * 10;
        word0 |= lo << shift;
        word1 |= hi << shift;
    }
    [word0, word1]
}

/// Decode for tests / debug (must match `compact_decode_position` in WGSL).
pub fn decode_position_u20x3(words: [u32; 2]) -> [f32; 3] {
    const SCALE: f32 = 32.0 / POSITION_MAX as f32;
    const OFFSET: f32 = -8.0;
    let mut out = [0u32; 3];
    for axis in 0..3 {
        let shift = (axis as u32) * 10;
        let lo = (words[0] >> shift) & 0x3FF;
        let hi = (words[1] >> shift) & 0x3FF;
        out[axis] = (hi << 10) | lo;
    }
    [
        out[0] as f32 * SCALE + OFFSET,
        out[1] as f32 * SCALE + OFFSET,
        out[2] as f32 * SCALE + OFFSET,
    ]
}

pub fn encode_baked_color_unorm8x4(tint_rgb: [f32; 3], shade_ao: f32) -> [u8; 4] {
    let m = shade_ao.clamp(0.0, 1.0);
    [
        (tint_rgb[0].clamp(0.0, 1.0) * m * 255.0).round() as u8,
        (tint_rgb[1].clamp(0.0, 1.0) * m * 255.0).round() as u8,
        (tint_rgb[2].clamp(0.0, 1.0) * m * 255.0).round() as u8,
        255,
    ]
}

pub fn encode_uv_packed_u16x2(uv: [f32; 2]) -> u32 {
    let u = (uv[0].clamp(0.0, 1.0) * 32767.0).round() as u32 & 0xFFFF;
    let v = (uv[1].clamp(0.0, 1.0) * 32767.0).round() as u32 & 0xFFFF;
    u | (v << 16)
}

pub fn encode_light_tile_u32(block_level: f32, sky_level: f32, atlas_tile: u16) -> u32 {
    let b = ((block_level.clamp(0.0, 15.0) / 15.0) * 255.0).round() as u32;
    let s = ((sky_level.clamp(0.0, 15.0) / 15.0) * 255.0).round() as u32;
    let light = (b << 8) | (s & 0xFF);
    (light << 16) | (atlas_tile as u32)
}

/// Append one compact vertex (20 bytes) to `out`.
pub fn push_compact_vertex(
    out: &mut Vec<u8>,
    position: [f32; 3],
    uv: [f32; 2],
    tint_rgb: [f32; 3],
    shade_ao: f32,
    block_light: f32,
    sky_light: f32,
    atlas_tile: u16,
) {
    debug_assert_eq!(out.len() % COMPACT_CHUNK_VERTEX_STRIDE as usize, 0);
    let w = encode_position_u20x3(position);
    out.extend_from_slice(&w[0].to_le_bytes());
    out.extend_from_slice(&w[1].to_le_bytes());
    out.extend_from_slice(&encode_baked_color_unorm8x4(tint_rgb, shade_ao));
    out.extend_from_slice(&encode_uv_packed_u16x2(uv).to_le_bytes());
    out.extend_from_slice(&encode_light_tile_u32(block_light, sky_light, atlas_tile).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_one_vertex_is_20_bytes() {
        let mut b = Vec::new();
        push_compact_vertex(
            &mut b,
            [1.0, 2.0, 3.0],
            [0.0, 1.0],
            [1.0, 0.5, 0.25],
            0.8,
            15.0,
            12.0,
            7,
        );
        assert_eq!(b.len(), 20);
    }

    #[test]
    fn position_roundtrip_chunk_local() {
        for p in [[0.0, 0.0, 0.0], [8.0, 8.0, 8.0], [15.99, 0.0, 16.0]] {
            let w = encode_position_u20x3(p);
            let d = decode_position_u20x3(w);
            for i in 0..3 {
                assert!(
                    (d[i] - p[i]).abs() < 0.05,
                    "p={p:?} d={d:?} words={w:?}"
                );
            }
        }
    }
}
