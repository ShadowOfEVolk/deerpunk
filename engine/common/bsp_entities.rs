//! GoldSrc BSP entities lump → key/value blocks.

use std::collections::HashMap;

/// One entity block from the entities lump (`{ "key" "value" ... }`).
pub type EntityKv = HashMap<String, String>;

/// Parse `"key" "value"` pairs inside `{ ... }` blocks from the entities lump.
pub fn parse_entity_blocks(entities: &str) -> Vec<EntityKv> {
    let mut out = Vec::new();
    for block in entities.split('{').skip(1) {
        let body = block.split('}').next().unwrap_or("");
        let kv = parse_quoted_pairs(body);
        if !kv.is_empty() {
            out.push(kv);
        }
    }
    out
}

fn parse_quoted_pairs(body: &str) -> EntityKv {
    let mut map = HashMap::new();
    // split('"'): odd indices are quoted tokens → key, value, key, value, …
    let parts: Vec<&str> = body.split('"').collect();
    let mut i = 1usize;
    while i + 2 < parts.len() {
        let key = parts[i];
        let val = parts[i + 2];
        if !key.is_empty() {
            map.insert(key.to_string(), val.to_string());
        }
        i += 4;
    }
    map
}

/// Parse `"x y z"` space-separated floats.
pub fn parse_vec3_str(s: &str) -> Option<[f32; 3]> {
    let mut it = s.split_whitespace();
    let x: f32 = it.next()?.parse().ok()?;
    let y: f32 = it.next()?.parse().ok()?;
    let z: f32 = it.next()?.parse().ok()?;
    Some([x, y, z])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_entities() {
        let raw = r#"
{
"classname" "worldspawn"
"wad" "a.wad;b.wad"
}
{
"classname" "info_player_start"
"origin" "16 32 48"
"angles" "0 90 0"
}
"#;
        let blocks = parse_entity_blocks(raw);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0].get("classname").map(String::as_str),
            Some("worldspawn")
        );
        assert_eq!(
            blocks[1].get("origin").map(String::as_str),
            Some("16 32 48")
        );
        assert_eq!(parse_vec3_str("16 32 48"), Some([16.0, 32.0, 48.0]));
    }
}
