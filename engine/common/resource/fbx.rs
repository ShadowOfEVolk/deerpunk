use crate::resource::mesh::Mesh;

pub fn parse_fbx(data: &[u8]) -> Result<Mesh, String> {
    if data.starts_with(b"Kaydara FBX Binary") {
        parse_fbx_binary(data)
    } else {
        let text = std::str::from_utf8(data).map_err(|_| "FBX must be binary (ASCII not supported)".to_string())?;
        if text.contains("FBX") || text.starts_with(';') {
            Err("ASCII FBX not supported, convert to binary".into())
        } else {
            Err("not an FBX file".into())
        }
    }
}

fn parse_fbx_binary(data: &[u8]) -> Result<Mesh, String> {
    if data.len() < 27 {
        return Err("FBX file too short".into());
    }

    let mut found_verts: Option<Vec<f64>> = None;
    let mut found_polys: Option<Vec<i32>> = None;

    let mut stack: Vec<usize> = vec![27];
    while let Some(offset) = stack.pop() {
        if offset + 13 > data.len() { continue; }
        let end = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap()) as usize;
        if end == 0 || end > data.len() { continue; }
        let np = u32::from_le_bytes(data[offset+4..offset+8].try_into().unwrap()) as usize;
        let nl = data[offset + 12] as usize;
        if offset + 13 + nl > data.len() { continue; }
        let name = std::str::from_utf8(&data[offset + 13..offset + 13 + nl]).unwrap_or("");

        let mut cur = offset + 13 + nl;
        for _ in 0..np {
            if cur + 1 > data.len() { break; }
            let dtype = data[cur];
            cur += 1;
            match dtype {
                b'Y' => cur += 2,
                b'C' => cur += 1,
                b'I' | b'F' => cur += 4,
                b'D' | b'L' => cur += 8,
                b'd' | b'i' | b'f' | b'l' => {
                    if cur + 4 > data.len() { break; }
                    let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize;
                    cur += 4;
                    let elem = match dtype { b'd' => 8, _ => 4 };
                    cur = (cur + cnt * elem).min(data.len());
                }
                b'b' => {
                    if cur + 4 > data.len() { break; }
                    let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize;
                    cur += 4 + cnt;
                }
                b'S' | b'R' => {
                    if cur + 4 > data.len() { break; }
                    let sl = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize;
                    cur += 4 + sl + 1;
                }
                _ => break,
            }
        }

        if name == "Vertices" && found_verts.is_none() {
            if let Some(arr) = read_f64_array(data, offset + 13 + nl, np) {
                if arr.len() % 3 == 0 && !arr.is_empty() {
                    found_verts = Some(arr);
                }
            }
        }
        if name == "PolygonVertexIndex" && found_polys.is_none() {
            if let Some(arr) = read_i32_array(data, offset + 13 + nl, np) {
                if !arr.is_empty() {
                    found_polys = Some(arr);
                }
            }
        }

        if found_verts.is_some() && found_polys.is_some() { break; }

        let mut child = cur;
        while child + 13 < end && child < data.len() {
            let child_end = u32::from_le_bytes(data[child..child+4].try_into().unwrap()) as usize;
            if child_end <= child || child_end > data.len() { child += 1; continue; }
            stack.push(child);
            child = child_end;
        }
    }

    let positions = found_verts.ok_or("no Vertices in FBX")?;
    let polys = found_polys.ok_or("no PolygonVertexIndex in FBX")?;

    if positions.len() < 3 || positions.len() % 3 != 0 {
        return Err(format!("bad vertices count: {}", positions.len()));
    }
    let mut verts = Vec::with_capacity(positions.len() / 3);
    for chunk in positions.chunks_exact(3) {
        verts.push([chunk[0] as f32, chunk[1] as f32, chunk[2] as f32]);
    }
    let normals = vec![[0.0, 0.0, 1.0]; verts.len()];
    let uvs = vec![[0.0, 0.0]; verts.len()];

    let mut indices = Vec::new();
    let mut i = 0usize;
    while i < polys.len() {
        let start = i;
        while i < polys.len() && polys[i] >= 0 { i += 1; }
        if i >= polys.len() { break; }
        let last = (-polys[i] - 1) as u32;
        let count = i - start + 1;
        if count >= 3 {
            let a = polys[start] as u32;
            for j in start + 1..i {
                indices.push(a);
                indices.push(polys[j] as u32);
                indices.push(last);
            }
        }
        i += 1;
    }

    Ok(Mesh {
        positions: verts,
        normals,
        uvs,
        lightmap_uvs: Vec::new(),
        lightmap_styles: Vec::new(),
        atlas_rects: Vec::new(),
        indices,
    })
}

fn read_f64_array(data: &[u8], prop_start: usize, np: usize) -> Option<Vec<f64>> {
    let mut cur = prop_start;
    for _ in 0..np {
        if cur + 1 > data.len() { return None; }
        let dtype = data[cur];
        cur += 1;
        match dtype {
            b'd' => {
                if cur + 4 > data.len() { return None; }
                let count = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize;
                cur += 4;
                if cur + count * 8 > data.len() { return None; }
                let mut arr = Vec::with_capacity(count);
                for j in 0..count {
                    arr.push(f64::from_le_bytes(data[cur + j*8..cur + j*8 + 8].try_into().unwrap()));
                }
                return Some(arr);
            }
            b'Y' => cur += 2, b'C' => cur += 1,
            b'I' | b'F' => cur += 4,
            b'D' | b'L' => cur += 8,
            b'i' | b'f' | b'l' => { if cur + 4 > data.len() { return None; } let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + cnt * 4; }
            b'b' => { if cur + 4 > data.len() { return None; } let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + cnt; }
            b'S' | b'R' => { if cur + 4 > data.len() { return None; } let sl = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + sl + 1; }
            _ => return None,
        }
    }
    None
}

fn read_i32_array(data: &[u8], prop_start: usize, np: usize) -> Option<Vec<i32>> {
    let mut cur = prop_start;
    for _ in 0..np {
        if cur + 1 > data.len() { return None; }
        let dtype = data[cur];
        cur += 1;
        match dtype {
            b'i' => {
                if cur + 4 > data.len() { return None; }
                let count = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize;
                cur += 4;
                if cur + count * 4 > data.len() { return None; }
                let mut arr = Vec::with_capacity(count);
                for j in 0..count {
                    arr.push(i32::from_le_bytes(data[cur + j*4..cur + j*4 + 4].try_into().unwrap()));
                }
                return Some(arr);
            }
            b'Y' => cur += 2, b'C' => cur += 1,
            b'I' | b'F' => cur += 4,
            b'D' | b'L' => cur += 8,
            b'd' => { if cur + 4 > data.len() { return None; } let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + cnt * 8; }
            b'f' | b'l' => { if cur + 4 > data.len() { return None; } let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + cnt * 4; }
            b'b' => { if cur + 4 > data.len() { return None; } let cnt = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + cnt; }
            b'S' | b'R' => { if cur + 4 > data.len() { return None; } let sl = u32::from_le_bytes(data[cur..cur+4].try_into().unwrap()) as usize; cur += 4 + sl + 1; }
            _ => return None,
        }
    }
    None
}
