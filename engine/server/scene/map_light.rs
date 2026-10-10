//! Apply GoldSrc light keyvalues from [`MapKv`] onto [`PointLight`].

use deerpunk_engine_common::components::{MapKv, PointLight};
use deerpunk_engine_common::world::{EntityId, World};

/// Override `PointLight` from map kv when present:
/// - `_light` / `rendercolor`: `"R G B"` or `"R G B brightness"` (0..255)
/// - `light` / `brightness`: scalar brightness
/// - `range` / `_distance`: falloff radius
pub fn apply_point_light_from_mapkv(world: &World, entity: EntityId) {
    let light_id = world.component_init::<PointLight>();
    let kv_id = world.component_init::<MapKv>();
    if !world.has(entity, light_id) {
        return;
    }
    let kv_ptr = world.get_raw(entity, kv_id) as *const MapKv;
    if kv_ptr.is_null() {
        return;
    }
    let kv = unsafe { &*kv_ptr };

    let mut light = world
        .get::<PointLight>(entity, light_id)
        .unwrap_or_default();

    if let Some(s) = kv.get("_light").or_else(|| kv.get("rendercolor")) {
        if let Some(parsed) = parse_light_color(s) {
            light.color = parsed.color;
            if let Some(b) = parsed.brightness {
                light.intensity = hl_brightness_to_intensity(b);
            }
        }
    }
    if let Some(s) = kv.get("light").or_else(|| kv.get("brightness")) {
        if let Ok(b) = s.trim().parse::<f32>() {
            light.intensity = hl_brightness_to_intensity(b);
        }
    }
    if let Some(s) = kv.get("range").or_else(|| kv.get("_distance")) {
        if let Ok(r) = s.trim().parse::<f32>() {
            light.range = r.max(1.0);
        }
    } else {
        // Heuristic: brighter → larger reach (HL-ish).
        light.range = (light.intensity * 900.0).clamp(200.0, 900.0);
    }

    world.set(entity, light_id, light);
}

struct ParsedLight {
    color: [f32; 3],
    brightness: Option<f32>,
}

fn hl_brightness_to_intensity(b: f32) -> f32 {
    // GoldSrc brightness often ~100–300; keep contribution dim (shader also *0.35).
    (b / 900.0).clamp(0.04, 0.6)
}

fn parse_light_color(s: &str) -> Option<ParsedLight> {
    let mut it = s.split_whitespace();
    let r: f32 = it.next()?.parse().ok()?;
    let g: f32 = it.next()?.parse().ok()?;
    let b: f32 = it.next()?.parse().ok()?;
    let brightness = it.next().and_then(|x| x.parse().ok());
    Some(ParsedLight {
        color: [
            (r / 255.0).clamp(0.0, 8.0),
            (g / 255.0).clamp(0.0, 8.0),
            (b / 255.0).clamp(0.0, 8.0),
        ],
        brightness,
    })
}
