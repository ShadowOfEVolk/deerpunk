//! GoldSrc BSP30 → world mesh + single albedo atlas.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use crate::atlas::{self, AtlasRect};
use crate::bsp_debug::{self, BspFaceDebug, BspWorldDebug};
use crate::bsp_lightmap::{self, FaceLightmap};
use crate::components::pending_bsp_atlas::PendingBspAtlas;
use crate::resource::mesh::Mesh;
use crate::wad::{self, RgbaImage, WadSet};

const LUMP_ENTITIES: usize = 0;
const LUMP_PLANES: usize = 1;
const LUMP_TEXTURES: usize = 2;
const LUMP_VERTEXES: usize = 3;
const LUMP_TEXINFO: usize = 6;
const LUMP_FACES: usize = 7;
const LUMP_LIGHTING: usize = 8;
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

/// BSP `lump models` entry (world = 0, brush ents = `*1`…).
#[derive(Clone, Copy, Debug)]
pub struct BspSubmodel {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub origin: [f32; 3],
}

impl BspSubmodel {
    /// World-space AABB center (brush ents usually have zero `origin`).
    pub fn center(self) -> [f32; 3] {
        [
            (self.mins[0] + self.maxs[0]) * 0.5,
            (self.mins[1] + self.maxs[1]) * 0.5,
            (self.mins[2] + self.maxs[2]) * 0.5,
        ]
    }
}

#[derive(Clone)]
pub struct BspWorldMesh {
    pub mesh: Mesh,
    pub pending: PendingBspAtlas,
    /// Per-style lightmap atlas (4 layers). None if BSP has no lighting.
    pub lightmap: Option<crate::bsp_lightmap::LightmapAtlasLayers>,
    /// Face pick meta for `r_bsp_debug` (worldspawn faces only).
    pub debug: BspWorldDebug,
    /// Raw entities lump (`{ "key" "value" ... }`).
    pub entities: String,
    /// All BSP models (index 0 = worldspawn hull).
    pub submodels: Vec<BspSubmodel>,
}

/// Alias: loaded BSP package (mesh + materials staging + entities lump).
pub type BspMap = BspWorldMesh;

/// Load world geometry + albedo atlas for GPU upload.
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
    let lighting = read_lump_bytes(data, lumps[LUMP_LIGHTING]);

    let miptex_infos = parse_miptex_dir(data, lumps[LUMP_TEXTURES])?;
    let (albedo_atlas, mip_map) = build_albedo_atlas(&miptex_infos, data, lumps[LUMP_TEXTURES], &wads);

    let world_range = world_face_range(models, faces.len());
    let world_faces = &faces[world_range.clone()];
    log::info!(
        "[bsp30] world faces {}..{} ({}), albedo_atlas={}x{}, lighting={} bytes",
        world_range.start,
        world_range.end,
        world_faces.len(),
        albedo_atlas.width,
        albedo_atlas.height,
        lighting.len()
    );

    struct BuiltFace {
        bsp_face: u32,
        tex_name: String,
        miptex: i32,
        styles: [u8; 4],
        lightofs: i32,
        verts: Vec<[f32; 3]>,
        normal: [f32; 3],
        s_coords: Vec<f32>,
        t_coords: Vec<f32>,
        atlas_rect: [f32; 4],
        tw: f32,
        th: f32,
        lm: Option<FaceLightmap>,
    }

    let mut built: Vec<BuiltFace> = Vec::new();
    let mut lm_slots: Vec<Option<FaceLightmap>> = Vec::new();
    let mut skipped = 0u32;

    for (local_i, face) in world_faces.iter().enumerate() {
        let bsp_face = (world_range.start + local_i) as u32;
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
        let Some((rect_norm, tw, th)) = mip_map.get(&(ti.miptex as i32)).copied() else {
            skipped += 1;
            continue;
        };
        let tex_name = miptex_infos
            .get(ti.miptex as usize)
            .map(|m| m.name.clone())
            .unwrap_or_default();

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
            let v0 = if se >= 0 { e.v[0] as usize } else { e.v[1] as usize };
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

        let s = ti.vecs[0];
        let t = ti.vecs[1];
        let mut s_coords = Vec::with_capacity(face_verts.len());
        let mut t_coords = Vec::with_capacity(face_verts.len());
        for p in &face_verts {
            s_coords.push(p[0] * s[0] + p[1] * s[1] + p[2] * s[2] + s[3]);
            t_coords.push(p[0] * t[0] + p[1] * t[1] + p[2] * t[2] + t[3]);
        }

        let lm = bsp_lightmap::calc_extents(&s_coords, &t_coords).and_then(|(texturemins, _ext, lw, lh)| {
            let mut face_lm =
                bsp_lightmap::extract_style_layers(lighting, face.lightofs, face.styles, lw, lh)?;
            face_lm.texturemins = texturemins;
            Some(face_lm)
        });
        lm_slots.push(lm.clone());
        built.push(BuiltFace {
            bsp_face,
            tex_name,
            miptex: ti.miptex,
            styles: face.styles,
            lightofs: face.lightofs,
            verts: face_verts,
            normal: n,
            s_coords,
            t_coords,
            atlas_rect: rect_norm,
            tw,
            th,
            lm,
        });
    }

    let lightmap_atlas = bsp_lightmap::pack_atlas_layers(&mut lm_slots);
    let (atlas_w, atlas_h) = lightmap_atlas
        .as_ref()
        .map(|i| (i.width, i.height))
        .unwrap_or((1, 1));
    for (i, slot) in lm_slots.iter().enumerate() {
        if let Some(b) = built.get_mut(i) {
            b.lm = slot.clone();
        }
    }
    let lm_faces = lm_slots.iter().filter(|s| s.is_some()).count();
    log::info!(
        "[bsp30] lightmaps faces_with_lm={lm_faces}/{} atlas={}x{} (4 style layers)",
        built.len(),
        atlas_w,
        atlas_h
    );

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut lightmap_uvs = Vec::new();
    let mut lightmap_styles = Vec::new();
    let mut atlas_rects = Vec::new();
    let mut indices = Vec::new();
    let mut debug_faces: Vec<BspFaceDebug> = Vec::with_capacity(built.len());
    let mut tri_face: Vec<u32> = Vec::new();

    for face in &built {
        let base = positions.len() as u32;
        let face_dbg_i = debug_faces.len() as u32;
        let first_tri = (indices.len() / 3) as u32;
        let styles = face
            .lm
            .as_ref()
            .map(|lm| bsp_lightmap::styles_as_f32(lm.styles))
            .unwrap_or([255.0, 255.0, 255.0, 255.0]);
        for (i, p) in face.verts.iter().enumerate() {
            positions.push(*p);
            normals.push(face.normal);
            let s = face.s_coords[i];
            let t = face.t_coords[i];
            // Tiled UV in texture space — shader does fract → atlas rect.
            uvs.push([s / face.tw, t / face.th]);
            atlas_rects.push(face.atlas_rect);
            let lm_uv = if let Some(ref lm) = face.lm {
                bsp_lightmap::lm_uv(lm, atlas_w, atlas_h, s, t)
            } else {
                [0.0, 0.0]
            };
            lightmap_uvs.push(lm_uv);
            lightmap_styles.push(styles);
        }
        for i in 1..(face.verts.len() as u32 - 1) {
            indices.extend_from_slice(&[base, base + i + 1, base + i]);
            tri_face.push(face_dbg_i);
        }
        let num_tris = (face.verts.len() as u32).saturating_sub(2);
        let mut style_avg_rgb = [[255u8; 3]; 4];
        let mut layers_rgb: [Option<std::sync::Arc<[u8]>>; 4] = [None, None, None, None];
        let (lm_w, lm_h, lm_atlas_x, lm_atlas_y, texturemins) = if let Some(ref lm) = face.lm {
            for slot in 0..4 {
                if lm.styles[slot] == 255 {
                    style_avg_rgb[slot] = [255, 255, 255];
                } else if let Some(ref rgb) = lm.layers_rgb[slot] {
                    style_avg_rgb[slot] = bsp_debug::avg_rgb(rgb);
                    layers_rgb[slot] = Some(std::sync::Arc::from(rgb.as_slice()));
                } else {
                    style_avg_rgb[slot] = [0, 0, 0];
                }
            }
            (
                lm.width,
                lm.height,
                lm.atlas_x,
                lm.atlas_y,
                lm.texturemins,
            )
        } else {
            (0, 0, 0, 0, [0, 0])
        };
        debug_faces.push(BspFaceDebug {
            bsp_face: face.bsp_face,
            tex_name: face.tex_name.clone(),
            miptex: face.miptex,
            styles: face.styles,
            lightofs: face.lightofs,
            lm_w,
            lm_h,
            lm_atlas_x,
            lm_atlas_y,
            texturemins,
            style_avg_rgb,
            layers_rgb,
            normal: face.normal,
            num_verts: face.verts.len() as u32,
            first_tri,
            num_tris,
        });
    }

    if positions.is_empty() {
        return Err("no geometry in world model".into());
    }

    log::info!(
        "[bsp30] mesh verts={} idx={} skipped={}",
        positions.len(),
        indices.len(),
        skipped
    );

    let submodels: Vec<BspSubmodel> = models
        .iter()
        .map(|m| BspSubmodel {
            mins: m.mins,
            maxs: m.maxs,
            origin: m.origin,
        })
        .collect();

    let (atlas_w, atlas_h, atlas_layers) = match &lightmap_atlas {
        Some(a) => (
            a.width,
            a.height,
            Some([
                std::sync::Arc::<[u8]>::from(a.layers[0].pixels.as_slice()),
                std::sync::Arc::<[u8]>::from(a.layers[1].pixels.as_slice()),
                std::sync::Arc::<[u8]>::from(a.layers[2].pixels.as_slice()),
                std::sync::Arc::<[u8]>::from(a.layers[3].pixels.as_slice()),
            ]),
        ),
        None => (0, 0, None),
    };

    let debug = BspWorldDebug {
        faces: std::sync::Arc::new(debug_faces),
        tri_face: std::sync::Arc::new(tri_face),
        atlas_w,
        atlas_h,
        atlas_layers,
    };

    Ok(BspWorldMesh {
        mesh: Mesh {
            positions,
            normals,
            uvs,
            lightmap_uvs,
            lightmap_styles,
            atlas_rects,
            indices,
        },
        pending: PendingBspAtlas::new(albedo_atlas),
        lightmap: lightmap_atlas,
        debug,
        entities,
        submodels,
    })
}

fn read_lump_bytes(data: &[u8], lump: Lump) -> &[u8] {
    let start = lump.offset as usize;
    let end = (start + lump.length as usize).min(data.len());
    if start >= end || start >= data.len() {
        return &[];
    }
    &data[start..end]
}

/// miptex index → (normalized atlas rect, tex_w, tex_h)
fn build_albedo_atlas(
    infos: &[MiptexInfo],
    data: &[u8],
    tex_lump: Lump,
    wads: &WadSet,
) -> (RgbaImage, HashMap<i32, ([f32; 4], f32, f32)>) {
    let lump_base = tex_lump.offset as usize;
    let mut images: Vec<RgbaImage> = Vec::new();
    let mut mip_indices: Vec<i32> = Vec::new();
    let mut sizes: Vec<(f32, f32)> = Vec::new();

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
        images.push(img);
        mip_indices.push(i as i32);
        sizes.push((tw, th));
    }

    if images.is_empty() {
        let missing = RgbaImage::checker(64, 64);
        images.push(missing);
        mip_indices.push(-1);
        sizes.push((64.0, 64.0));
    }

    let refs: Vec<&RgbaImage> = images.iter().collect();
    let packed = atlas::pack(&refs, 2);
    let aw = packed.image.width;
    let ah = packed.image.height;

    let mut map: HashMap<i32, ([f32; 4], f32, f32)> = HashMap::new();
    for (idx, &mip) in mip_indices.iter().enumerate() {
        if mip < 0 {
            continue;
        }
        let rect: AtlasRect = packed.rects[idx];
        let (tw, th) = sizes[idx];
        map.insert(mip, (atlas::rect_norm(&rect, aw, ah), tw, th));
    }

    // If only placeholder was packed, still map nothing — faces without mip skip.
    // Provide a full-atlas fallback for any face that somehow references missing.
    if map.is_empty() && !packed.rects.is_empty() {
        let r = atlas::rect_norm(&packed.rects[0], aw, ah);
        map.insert(0, (r, 64.0, 64.0));
    }

    (packed.image, map)
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
    fn load_oldlifehome_atlas() {
        let bsp = Path::new(r"D:/projects/darkdemo/content/darkdemo/maps/oldlifehome.bsp");
        let gamedir = Path::new(r"D:/projects/darkdemo/content/darkdemo");
        if !bsp.is_file() {
            return;
        }
        let m = load_world_mesh(bsp, gamedir).expect("bsp");
        assert!(m.mesh.positions.len() > 100);
        assert!(m.pending.image.is_some());
        let img = m.pending.image.as_ref().unwrap();
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(m.mesh.atlas_rects.len(), m.mesh.positions.len());
        eprintln!(
            "atlas={}x{} verts={} idx={}",
            img.width,
            img.height,
            m.mesh.positions.len(),
            m.mesh.indices.len()
        );
    }
}
