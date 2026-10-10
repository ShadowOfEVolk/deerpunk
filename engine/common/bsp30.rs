//! GoldSrc BSP30 → world mesh + texture-array buckets (submeshes).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use crate::bsp_buckets::{self, BUCKET_SIZES};
use crate::components::pending_bsp_arrays::{PendingBspArrays, PendingMeshPart, PendingTexBucket};
use crate::resource::mesh::Mesh;
use crate::wad::{self, RgbaImage, WadSet};

const LUMP_ENTITIES: usize = 0;
const LUMP_PLANES: usize = 1;
const LUMP_TEXTURES: usize = 2;
const LUMP_VERTEXES: usize = 3;
const LUMP_TEXINFO: usize = 6;
const LUMP_FACES: usize = 7;
const LUMP_EDGES: usize = 12;
const LUMP_SURFEDGES: usize = 13;
const LUMP_MODELS: usize = 14;

#[derive(Clone, Copy)]
struct Lump {
    offset: u32,
    length: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Plane {
    normal: [f32; 3],
    dist: f32,
    type_: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Face {
    planenum: i16,
    side: i16,
    firstedge: i32,
    numedges: i16,
    texinfo: i16,
    styles: [u8; 4],
    lightofs: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Edge {
    v: [u16; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TexInfo {
    vecs: [[f32; 4]; 2],
    miptex: i32,
    flags: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Model {
    mins: [f32; 3],
    maxs: [f32; 3],
    origin: [f32; 3],
    headnode: [i32; 4],
    visleafs: i32,
    firstface: i32,
    numfaces: i32,
}

const MAX_FACE_EDGES: i16 = 64;

#[derive(Clone)]
pub struct BspWorldMesh {
    pub mesh: Mesh,
    pub pending: PendingBspArrays,
    /// Raw entities lump (`{ "key" "value" ... }`).
    pub entities: String,
}

/// Alias: loaded BSP package (mesh + materials staging + entities lump).
pub type BspMap = BspWorldMesh;

/// Load world geometry + texture buckets for array upload.
pub fn load_world_mesh(bsp_path: &Path, gamedir: &Path) -> Result<BspWorldMesh, String> {
    let data = std::fs::read(bsp_path).map_err(|e| format!("{}: {e}", bsp_path.display()))?;
    mesh_from_bytes(&data, gamedir).map_err(|e| format!("{}: {e}", bsp_path.display()))
}

pub fn mesh_from_bytes(data: &[u8], gamedir: &Path) -> Result<BspWorldMesh, String> {
    if data.len() < 4 + 15 * 8 {
        return Err("file too small for BSP header".into());
    }
    let mut cur = Cursor::new(data);
    let version = read_u32(&mut cur)?;
    if version != 30 {
        return Err(format!("expected BSP30, got version {version}"));
    }
    let mut lumps = [Lump {
        offset: 0,
        length: 0,
    }; 15];
    for lump in &mut lumps {
        lump.offset = read_u32(&mut cur)?;
        lump.length = read_u32(&mut cur)?;
    }

    let entities = read_entities(data, lumps[LUMP_ENTITIES]);
    let wad_names = wad::parse_wad_list_from_entities(&entities);
    let wads = WadSet::from_gamedir(gamedir, &wad_names);

    let planes: &[Plane] = read_lump_slice(data, lumps[LUMP_PLANES])?;
    let vertices: &[[f32; 3]] = read_lump_slice(data, lumps[LUMP_VERTEXES])?;
    let texinfos: &[TexInfo] = read_lump_slice(data, lumps[LUMP_TEXINFO])?;
    let faces: &[Face] = read_lump_slice(data, lumps[LUMP_FACES])?;
    let edges: &[Edge] = read_lump_slice(data, lumps[LUMP_EDGES])?;
    let surfedges: &[i32] = read_lump_slice(data, lumps[LUMP_SURFEDGES])?;
    let models: &[Model] = read_lump_slice(data, lumps[LUMP_MODELS])?;

    let miptex_infos = parse_miptex_dir(data, lumps[LUMP_TEXTURES])?;
    // mip index → (bucket_slot in BUCKET_SIZES, layer, tex_w, tex_h for UV)
    let (buckets, mip_map) = build_buckets(&miptex_infos, data, lumps[LUMP_TEXTURES], &wads);

    let world_range = world_face_range(models, faces.len());
    let world_faces = &faces[world_range.clone()];
    log::info!(
        "[bsp30] world faces {}..{} ({}), buckets={}",
        world_range.start,
        world_range.end,
        world_faces.len(),
        buckets.len()
    );

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut tex_layers = Vec::new();
    // One draw per (bucket, layer): texture_2d_array index must be uniform per draw.
    let mut bins: HashMap<(u32, u32), Vec<[u32; 3]>> = HashMap::new();
    let mut skipped = 0u32;

    for face in world_faces {
        if !face_header_ok(face, planes.len(), texinfos.len(), surfedges.len()) {
            skipped += 1;
            continue;
        }
        let Some(plane) = planes.get(face.planenum as usize) else {
            skipped += 1;
            continue;
        };
        let mut n = plane.normal;
        if face.side != 0 {
            n[0] = -n[0];
            n[1] = -n[1];
            n[2] = -n[2];
        }

        let Some(ti) = texinfos.get(face.texinfo as usize) else {
            skipped += 1;
            continue;
        };
        // Skip faces whose miptex was omitted (tool textures / empty).
        let Some((bucket_idx, layer, tw, th)) = mip_map.get(&(ti.miptex as i32)).copied() else {
            skipped += 1;
            continue;
        };

        let first = face.firstedge as usize;
        let count = face.numedges as usize;
        let mut face_verts: Vec<[f32; 3]> = Vec::with_capacity(count);
        let mut face_ok = true;
        for i in 0..count {
            let Some(&se) = surfedges.get(first + i) else {
                face_ok = false;
                break;
            };
            let edge_idx = if se >= 0 { se as usize } else { (-se) as usize };
            let Some(e) = edges.get(edge_idx) else {
                face_ok = false;
                break;
            };
            let v0 = if se >= 0 {
                e.v[0] as usize
            } else {
                e.v[1] as usize
            };
            let Some(&p) = vertices.get(v0) else {
                face_ok = false;
                break;
            };
            face_verts.push(p);
        }
        if !face_ok || face_verts.len() < 3 {
            skipped += 1;
            continue;
        }

        let base = positions.len() as u32;
        let s = ti.vecs[0];
        let t = ti.vecs[1];
        for p in &face_verts {
            positions.push(*p);
            normals.push(n);
            let u = p[0] * s[0] + p[1] * s[1] + p[2] * s[2] + s[3];
            let v = p[0] * t[0] + p[1] * t[1] + p[2] * t[2] + t[3];
            // True tiling UVs — Repeat sampler on the array layer.
            uvs.push([u / tw, v / th]);
            tex_layers.push(layer as f32);
        }
        let bin = bins.entry((bucket_idx, layer)).or_default();
        for i in 1..(face_verts.len() as u32 - 1) {
            // CW→CCW for wgpu backface cull
            bin.push([base, base + i + 1, base + i]);
        }
    }

    if positions.is_empty() {
        return Err("no geometry in world model".into());
    }

    let mut indices = Vec::new();
    let mut parts = Vec::new();
    let mut keys: Vec<(u32, u32)> = bins.keys().copied().collect();
    keys.sort_unstable();
    for (bucket_idx, layer) in keys {
        let tris = bins.remove(&(bucket_idx, layer)).unwrap();
        let first_index = indices.len() as u32;
        for t in &tris {
            indices.extend_from_slice(t);
        }
        let index_count = indices.len() as u32 - first_index;
        if index_count > 0 {
            parts.push(PendingMeshPart {
                first_index,
                index_count,
                bucket: bucket_idx,
                layer,
            });
        }
    }

    log::info!(
        "[bsp30] mesh verts={} idx={} submesh_draws={} skipped={}",
        positions.len(),
        indices.len(),
        parts.len(),
        skipped
    );

    Ok(BspWorldMesh {
        mesh: Mesh {
            positions,
            normals,
            uvs,
            tex_layers,
            indices,
        },
        pending: PendingBspArrays { buckets, parts },
        entities,
    })
}

/// mip → (bucket_index, layer, uv_width, uv_height)
fn build_buckets(
    infos: &[MiptexInfo],
    data: &[u8],
    tex_lump: Lump,
    wads: &WadSet,
) -> (Vec<PendingTexBucket>, HashMap<i32, (u32, u32, f32, f32)>) {
    let lump_base = tex_lump.offset as usize;
    let mut bucket_layers: Vec<Vec<RgbaImage>> = BUCKET_SIZES.iter().map(|_| Vec::new()).collect();
    let mut map: HashMap<i32, (u32, u32, f32, f32)> = HashMap::new();

    // Placeholder layer 0 in every used bucket
    let missing = RgbaImage::checker(64, 64);

    for (i, info) in infos.iter().enumerate() {
        if info.name.is_empty() || info.lump_ofs.is_none() {
            continue;
        }
        let name = &info.name;
        let lower = wad::normalize_tex_name(name);
        if skip_tool_texture(&lower) {
            continue;
        }

        let img = if info.embedded {
            let o = lump_base + info.lump_ofs.unwrap();
            let end = (tex_lump.offset + tex_lump.length) as usize;
            match wad::decode_miptex(&data[o..end.min(data.len())]) {
                Ok(im) => im,
                Err(_) => wads
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| RgbaImage::magenta(info.width.max(8), info.height.max(8))),
            }
        } else if let Some(im) = wads.get(name) {
            im.clone()
        } else {
            log::warn!("[bsp30] missing texture '{name}'");
            RgbaImage::magenta(info.width.max(8).min(256), info.height.max(8).min(256))
        };

        let tw = img.width.max(1) as f32;
        let th = img.height.max(1) as f32;
        let bucket_size = bsp_buckets::pick_bucket(img.width, img.height);
        let bucket_idx = BUCKET_SIZES
            .iter()
            .position(|&b| b == bucket_size)
            .unwrap_or(0) as u32;

        // Ensure placeholder at layer 0
        if bucket_layers[bucket_idx as usize].is_empty() {
            bucket_layers[bucket_idx as usize]
                .push(bsp_buckets::resize_to_bucket(&missing, bucket_size));
        }

        let layer = bucket_layers[bucket_idx as usize].len() as u32;
        bucket_layers[bucket_idx as usize].push(bsp_buckets::resize_to_bucket(&img, bucket_size));
        map.insert(i as i32, (bucket_idx, layer, tw, th));
    }

    // If nothing loaded, still provide one bucket so draws work.
    if map.is_empty() {
        bucket_layers[0].push(bsp_buckets::resize_to_bucket(&missing, 64));
    }

    let mut buckets = Vec::new();
    let mut remap_bucket: HashMap<u32, u32> = HashMap::new(); // old idx → packed idx
    for (i, layers) in bucket_layers.into_iter().enumerate() {
        if layers.is_empty() {
            continue;
        }
        let packed = buckets.len() as u32;
        remap_bucket.insert(i as u32, packed);
        buckets.push(PendingTexBucket {
            size: BUCKET_SIZES[i],
            layers,
        });
    }

    // Remap bucket indices in map after packing empty ones out.
    for v in map.values_mut() {
        if let Some(&nb) = remap_bucket.get(&v.0) {
            v.0 = nb;
        }
    }

    (buckets, map)
}

struct MiptexInfo {
    name: String,
    width: u32,
    height: u32,
    lump_ofs: Option<usize>,
    embedded: bool,
}

fn parse_miptex_dir(data: &[u8], lump: Lump) -> Result<Vec<MiptexInfo>, String> {
    let start = lump.offset as usize;
    let end = start + lump.length as usize;
    let bytes = data.get(start..end).ok_or("textures lump OOB")?;
    if bytes.len() < 4 {
        return Ok(Vec::new());
    }
    let num = i32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if num <= 0 {
        return Ok(Vec::new());
    }
    let num = num as usize;
    if 4 + num * 4 > bytes.len() {
        return Err("miptex ofs table OOB".into());
    }
    let mut out = Vec::with_capacity(num);
    for i in 0..num {
        let ofs = i32::from_le_bytes(bytes[4 + i * 4..8 + i * 4].try_into().unwrap());
        if ofs < 0 {
            out.push(MiptexInfo {
                name: String::new(),
                width: 16,
                height: 16,
                lump_ofs: None,
                embedded: false,
            });
            continue;
        }
        let o = ofs as usize;
        if o + 40 > bytes.len() {
            out.push(MiptexInfo {
                name: format!("bad{i}"),
                width: 16,
                height: 16,
                lump_ofs: None,
                embedded: false,
            });
            continue;
        }
        let name = cstr16(&bytes[o..o + 16]);
        let width = u32::from_le_bytes(bytes[o + 16..o + 20].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[o + 20..o + 24].try_into().unwrap());
        let off0 = u32::from_le_bytes(bytes[o + 24..o + 28].try_into().unwrap());
        out.push(MiptexInfo {
            name,
            width: width.max(1),
            height: height.max(1),
            lump_ofs: Some(o),
            embedded: off0 != 0,
        });
    }
    Ok(out)
}

fn skip_tool_texture(name: &str) -> bool {
    name.starts_with("sky")
        || name == "clip"
        || name == "trigger"
        || name == "hint"
        || name == "skip"
        || name == "null"
        || name == "aaatrigger"
        || name == "origin"
        || name == "bbox"
        || name.starts_with('!')
}

fn model_face_range_ok(model: &Model, faces_len: usize) -> bool {
    if model.firstface < 0 || model.numfaces <= 0 {
        return false;
    }
    let start = model.firstface as usize;
    let end = start.saturating_add(model.numfaces as usize);
    end <= faces_len
}

fn world_face_range(models: &[Model], faces_len: usize) -> std::ops::Range<usize> {
    if let Some(m0) = models.first() {
        if model_face_range_ok(m0, faces_len) {
            let start = m0.firstface as usize;
            return start..start + m0.numfaces as usize;
        }
        log::warn!(
            "[bsp30] model0 face range invalid (firstface={}, numfaces={}); inferring from submodels",
            m0.firstface,
            m0.numfaces
        );
    }
    let mut end = faces_len;
    for m in models.iter().skip(1) {
        if model_face_range_ok(m, faces_len) && m.numfaces <= MAX_FACE_EDGES as i32 {
            end = end.min(m.firstface as usize);
        }
    }
    0..end
}

fn face_header_ok(face: &Face, plane_count: usize, tex_count: usize, surfedge_count: usize) -> bool {
    if face.numedges < 3 || face.numedges > MAX_FACE_EDGES {
        return false;
    }
    if face.side != 0 && face.side != 1 {
        return false;
    }
    if face.planenum < 0 || face.planenum as usize >= plane_count {
        return false;
    }
    if face.texinfo < 0 || face.texinfo as usize >= tex_count {
        return false;
    }
    if face.firstedge < 0 {
        return false;
    }
    let first = face.firstedge as usize;
    let n = face.numedges as usize;
    first.checked_add(n).is_some_and(|end| end <= surfedge_count)
}

fn read_entities(data: &[u8], lump: Lump) -> String {
    let start = lump.offset as usize;
    let end = (start + lump.length as usize).min(data.len());
    if start >= end {
        return String::new();
    }
    String::from_utf8_lossy(&data[start..end]).into_owned()
}

fn cstr16(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn read_u32(r: &mut impl Read) -> Result<u32, String> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u32::from_le_bytes(b))
}

fn read_lump_slice<T: Copy>(data: &[u8], lump: Lump) -> Result<&[T], String> {
    let start = lump.offset as usize;
    let end = start
        .checked_add(lump.length as usize)
        .ok_or("lump overflow")?;
    let bytes = data
        .get(start..end)
        .ok_or_else(|| format!("lump out of range {}..{}", start, end))?;
    let sz = std::mem::size_of::<T>();
    if sz == 0 || bytes.len() % sz != 0 {
        return Err(format!("lump size {} not multiple of {}", bytes.len(), sz));
    }
    let count = bytes.len() / sz;
    Ok(unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const T, count) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_oldlifehome_buckets() {
        let bsp = Path::new(r"D:/projects/darkdemo/content/darkdemo/maps/oldlifehome.bsp");
        let gamedir = Path::new(r"D:/projects/darkdemo/content/darkdemo");
        if !bsp.is_file() {
            return;
        }
        let m = load_world_mesh(bsp, gamedir).expect("bsp");
        assert!(m.mesh.positions.len() > 100);
        assert!(!m.pending.buckets.is_empty());
        assert!(!m.pending.parts.is_empty());
        eprintln!(
            "buckets={} parts={} verts={}",
            m.pending.buckets.len(),
            m.pending.parts.len(),
            m.mesh.positions.len()
        );
        for (i, b) in m.pending.buckets.iter().enumerate() {
            eprintln!("  bucket{i} size={} layers={}", b.size, b.layers.len());
        }
    }
}
