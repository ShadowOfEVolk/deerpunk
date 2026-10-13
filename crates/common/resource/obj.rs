use std::path::{Path, PathBuf};

use crate::resource::mesh::Mesh;
use crate::wad::RgbaImage;

pub fn parse_obj(data: &str) -> Result<Mesh, String> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();

    let mut out_positions: Vec<[f32; 3]> = Vec::new();
    let mut out_normals: Vec<[f32; 3]> = Vec::new();
    let mut out_uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tag = match parts.next() {
            Some(t) => t,
            None => continue,
        };
        match tag {
            "v" => {
                let x = parts.next().and_then(|s| s.parse::<f32>().ok()).ok_or_else(|| format!("bad vertex: {line}"))?;
                let y = parts.next().and_then(|s| s.parse::<f32>().ok()).ok_or_else(|| format!("bad vertex: {line}"))?;
                let z = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                positions.push([x, y, z]);
            }
            "vn" => {
                let x = parts.next().and_then(|s| s.parse::<f32>().ok()).ok_or_else(|| format!("bad normal: {line}"))?;
                let y = parts.next().and_then(|s| s.parse::<f32>().ok()).ok_or_else(|| format!("bad normal: {line}"))?;
                let z = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                normals.push([x, y, z]);
            }
            "vt" => {
                let u = parts.next().and_then(|s| s.parse::<f32>().ok()).ok_or_else(|| format!("bad texcoord: {line}"))?;
                let v = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                uvs.push([u, v]);
            }
            "f" => {
                let verts: Vec<&str> = parts.collect();
                if verts.len() < 3 {
                    return Err(format!("face with <3 verts: {line}"));
                }
                let parse_ref = |s: &str| -> Result<(usize, Option<usize>, Option<usize>), String> {
                    let mut it = s.split('/');
                    let v = it.next().and_then(|s| s.parse::<i32>().ok()).ok_or_else(|| format!("bad face ref: {s}"))?;
                    let vt = it.next().and_then(|s| s.parse::<i32>().ok());
                    let vn = it.next().and_then(|s| s.parse::<i32>().ok());
                    Ok((v as usize, vt.map(|i| i as usize), vn.map(|i| i as usize)))
                };
                let (v0, t0, n0) = parse_ref(verts[0])?;
                let (v1, t1, n1) = parse_ref(verts[1])?;
                for i in 2..verts.len() {
                    let (v2, t2, n2) = parse_ref(verts[i])?;
                    for &(vi, ti, ni) in &[(v0, t0, n0), (v1, t1, n1), (v2, t2, n2)] {
                        let pos_idx = if vi > 0 { vi - 1 } else { positions.len().wrapping_sub(vi) };
                        let pos = *positions.get(pos_idx).ok_or_else(|| format!("vertex index {vi} out of range"))?;
                        let norm = ni.and_then(|ni| {
                            let idx = if ni > 0 { ni - 1 } else { normals.len().wrapping_sub(ni) };
                            normals.get(idx).copied()
                        }).unwrap_or(crate::coord::UP);
                        let uv = ti.and_then(|ti| {
                            let idx = if ti > 0 { ti - 1 } else { uvs.len().wrapping_sub(ti) };
                            uvs.get(idx).copied()
                        }).unwrap_or([0.0, 0.0]);
                        out_positions.push(pos);
                        out_normals.push(norm);
                        out_uvs.push(uv);
                        indices.push((out_positions.len() - 1) as u32);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Mesh {
        positions: out_positions,
        normals: out_normals,
        uvs: out_uvs,
        lightmap_uvs: Vec::new(),
        lightmap_styles: Vec::new(),
        atlas_rects: Vec::new(),
        indices,
    })
}

/// First `mtllib` name in an OBJ (basename or relative path).
pub fn find_mtllib(obj_data: &str) -> Option<String> {
    for line in obj_data.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("mtllib") {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// First `map_Kd` path from MTL text (single diffuse, first material that has one).
pub fn parse_mtl_first_map_kd(mtl_data: &str) -> Option<String> {
    for line in mtl_data.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("map_Kd")
            .or_else(|| line.strip_prefix("map_kd"))
        else {
            continue;
        };
        // Skip option tokens (`-bm 1`, `-s u v`, …); path may contain spaces.
        let path = strip_mtl_map_options(rest.trim());
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
}

fn strip_mtl_map_options(s: &str) -> &str {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        if !tokens[i].starts_with('-') {
            let start = s.find(tokens[i]).unwrap_or(0);
            return s[start..].trim();
        }
        let argc = match tokens[i] {
            "-bm" | "-boost" | "-mm" => 1,
            "-o" | "-s" | "-t" => 3,
            _ => 1,
        };
        i += 1 + argc;
    }
    ""
}

/// Resolve `map_Kd` against the OBJ directory; fallback to basename under `textures/`.
pub fn resolve_map_kd(obj_path: &Path, map_kd: &str) -> Option<PathBuf> {
    let obj_dir = obj_path.parent()?;
    let map_path = Path::new(map_kd);
    let basename = map_path.file_name()?;

    let mut candidates: Vec<PathBuf> = vec![
        obj_dir.join(map_kd),
        obj_dir.join(basename),
        obj_dir.join("textures").join(basename),
    ];
    if let Some(parent) = obj_dir.parent() {
        candidates.push(parent.join("textures").join(basename));
        candidates.push(parent.join(basename));
    }
    let mut dir = Some(obj_dir);
    for _ in 0..4 {
        let Some(d) = dir else { break };
        candidates.push(d.join("textures").join(basename));
        dir = d.parent();
    }

    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Load first diffuse from OBJ's MTL into RGBA8 (for [`PendingAlbedoAtlas`]).
pub fn load_obj_diffuse_albedo(obj_path: &Path) -> Option<RgbaImage> {
    let obj_data = std::fs::read_to_string(obj_path).ok()?;
    let mtllib = find_mtllib(&obj_data)?;
    let obj_dir = obj_path.parent()?;
    let mtl_path = obj_dir.join(&mtllib);
    let mtl_data = std::fs::read_to_string(&mtl_path).ok()?;
    let map_kd = parse_mtl_first_map_kd(&mtl_data)?;
    let tex_path = resolve_map_kd(obj_path, &map_kd)?;
    match image::open(&tex_path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            log::info!(
                "[obj] albedo {} -> {}x{} ({})",
                tex_path.display(),
                w,
                h,
                obj_path.display()
            );
            Some(RgbaImage {
                width: w,
                height: h,
                pixels: rgba.into_raw(),
            })
        }
        Err(e) => {
            log::warn!(
                "[obj] failed to load albedo '{}': {}",
                tex_path.display(),
                e
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_triangle() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3";
        let mesh = parse_obj(obj).unwrap();
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
    }

    #[test]
    fn parse_with_uvs_and_normals() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
vn 0 0 1
vn 0 0 1
vn 0 0 1
f 1/1/1 2/2/2 3/3/3";
        let mesh = parse_obj(obj).unwrap();
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.uvs.len(), 3);
        assert_eq!(mesh.normals.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
    }

    #[test]
    fn parse_quad_becomes_two_tris() {
        let obj = "\
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4";
        let mesh = parse_obj(obj).unwrap();
        assert_eq!(mesh.positions.len(), 6);
        assert_eq!(mesh.indices.len(), 6);
    }

    #[test]
    fn mtl_map_kd_with_spaces() {
        let mtl = "newmtl X\nmap_Kd Neuer Ordner (15)/textures/Dragon_Bump_Col2.jpg\n";
        assert_eq!(
            parse_mtl_first_map_kd(mtl).as_deref(),
            Some("Neuer Ordner (15)/textures/Dragon_Bump_Col2.jpg")
        );
    }

    #[test]
    fn mtl_map_kd_skips_options() {
        let mtl = "map_Bump -bm 1.000000 foo/bar.png\nmap_Kd -s 1 1 1 tex/diff.jpg\n";
        assert_eq!(
            parse_mtl_first_map_kd(mtl).as_deref(),
            Some("tex/diff.jpg")
        );
    }

    #[test]
    fn find_mtllib_line() {
        let obj = "mtllib washme.mtl\nv 0 0 0\n";
        assert_eq!(find_mtllib(obj).as_deref(), Some("washme.mtl"));
    }
}