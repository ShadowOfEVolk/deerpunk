use crate::resource::mesh::Mesh;

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
        indices,
    })
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
}