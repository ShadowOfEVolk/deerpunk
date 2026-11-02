// Engine built-in: screen-space dither threshold in [0, 1).
// Load via `#include "internal://shaders/inc/dither.wgsl"` or `internal://shaders/inc/dither.wgsl` URI.
//
// Defines (also accept legacy OHMYBLOCK_* names):
//   (none)                 — 4×4 Bayer (default)
//   DEERPUNK_DITHER_BLUE   / OHMYBLOCK_DITHER_BLUE   — interleaved gradient
//   DEERPUNK_DITHER_WHITE  / OHMYBLOCK_DITHER_WHITE  — per-pixel hash

fn bayer4(ix: u32, iy: u32) -> f32 {
    let p = iy * 4u + ix;
    switch p {
        case 0u: { return 0.0 / 16.0; }
        case 1u: { return 8.0 / 16.0; }
        case 2u: { return 2.0 / 16.0; }
        case 3u: { return 10.0 / 16.0; }
        case 4u: { return 12.0 / 16.0; }
        case 5u: { return 4.0 / 16.0; }
        case 6u: { return 14.0 / 16.0; }
        case 7u: { return 6.0 / 16.0; }
        case 8u: { return 3.0 / 16.0; }
        case 9u: { return 11.0 / 16.0; }
        case 10u: { return 1.0 / 16.0; }
        case 11u: { return 9.0 / 16.0; }
        case 12u: { return 15.0 / 16.0; }
        case 13u: { return 7.0 / 16.0; }
        case 14u: { return 13.0 / 16.0; }
        default: { return 5.0 / 16.0; }
    }
}

fn dither_hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn dither_interleaved_gradient(pix: vec2<f32>) -> f32 {
    let magic = vec3<f32>(0.06711056, 0.00583715, 52.9829189);
    return fract(magic.z * fract(dot(pix, magic.xy)));
}

#ifdef DEERPUNK_DITHER_WHITE
fn dither_threshold(pix_xy: vec2<f32>) -> f32 {
    return dither_hash21(floor(pix_xy));
}
#else
#ifdef OHMYBLOCK_DITHER_WHITE
fn dither_threshold(pix_xy: vec2<f32>) -> f32 {
    return dither_hash21(floor(pix_xy));
}
#else
#ifdef DEERPUNK_DITHER_BLUE
fn dither_threshold(pix_xy: vec2<f32>) -> f32 {
    return dither_interleaved_gradient(floor(pix_xy) + 0.5);
}
#else
#ifdef OHMYBLOCK_DITHER_BLUE
fn dither_threshold(pix_xy: vec2<f32>) -> f32 {
    return dither_interleaved_gradient(floor(pix_xy) + 0.5);
}
#else
fn dither_threshold(pix_xy: vec2<f32>) -> f32 {
    let ix = u32(pix_xy.x) & 3u;
    let iy = u32(pix_xy.y) & 3u;
    return bayer4(ix, iy);
}
#endif
#endif
#endif
#endif

fn dither_keep(pix_xy: vec2<f32>, coverage: f32) -> bool {
    if (coverage >= 1.0) {
        return true;
    }
    if (coverage <= 0.0) {
        return false;
    }
    return dither_threshold(pix_xy) < coverage;
}
