//! **DPK1** — binary layout metadata for `#[repr(C)]` component structs.
//!
//! One blob per component type: field names, offsets, sizes, wire kinds, flags.
//! Inspector, replication, and runtime patch should read this table (not ad-hoc enums).
//!
//! Optional bridge: [`layout_to_json_schema`] maps the same fields to JSON Schema.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::world::ComponentId;

pub const MAGIC: &[u8; 4] = b"DPK1";
pub const VERSION: u8 = 1;

/// Stable per-field id inside one component type (`0..n-1` or assigned explicitly).
pub type FieldId = u16;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireKind {
    Void = 0,
    Bool = 1,
    U8 = 2,
    U16 = 3,
    U32 = 4,
    U64 = 5,
    I32 = 6,
    F32 = 7,
    /// Three `f32` (12 bytes).
    Vec3 = 8,
    /// Four `f32` (16 bytes).
    Quat = 9,
    /// Heap / indirect (size in layout is pointer width; patch via gamelib hook).
    Indirect = 10,
}

impl WireKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Void,
            1 => Self::Bool,
            2 => Self::U8,
            3 => Self::U16,
            4 => Self::U32,
            5 => Self::U64,
            6 => Self::I32,
            7 => Self::F32,
            8 => Self::Vec3,
            9 => Self::Quat,
            10 => Self::Indirect,
            _ => return None,
        })
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct FieldFlags: u8 {
        const PREFAB = 1 << 0;
        const INSPECT = 1 << 1;
        const REPLICATE_HOT = 1 << 2;
        const REPLICATE_COLD = 1 << 3;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMeta {
    pub id: FieldId,
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub align: u32,
    pub wire: WireKind,
    pub flags: FieldFlags,
    pub doc: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutMeta {
    pub name: String,
    pub size: u32,
    pub align: u32,
    pub fields: Vec<FieldMeta>,
}

impl LayoutMeta {
    pub fn field(&self, name: &str) -> Option<&FieldMeta> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn field_by_id(&self, id: FieldId) -> Option<&FieldMeta> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// `Component.field` or `field` when component name matches.
    pub fn resolve_path(&self, path: &str) -> Option<&FieldMeta> {
        let path = path.trim();
        if let Some((comp, field)) = path.split_once('.') {
            if comp != self.name {
                return None;
            }
            return self.field(field);
        }
        self.field(path)
    }
}

#[derive(Clone)]
pub struct RegisteredLayout {
    pub component_id: ComponentId,
    pub layout: LayoutMeta,
}

pub struct LayoutMetaRegistry {
    by_name: Mutex<HashMap<String, RegisteredLayout>>,
    by_id: Mutex<HashMap<ComponentId, RegisteredLayout>>,
}

impl LayoutMetaRegistry {
    pub fn new() -> Self {
        Self {
            by_name: Mutex::new(HashMap::new()),
            by_id: Mutex::new(HashMap::new()),
        }
    }

    pub fn register(
        &self,
        component_id: ComponentId,
        layout: LayoutMeta,
    ) -> Result<(), String> {
        let entry = RegisteredLayout {
            component_id,
            layout,
        };
        let name = entry.layout.name.clone();
        let mut by_name = self
            .by_name
            .lock()
            .map_err(|_| "layout registry lock poisoned".to_string())?;
        let mut by_id = self
            .by_id
            .lock()
            .map_err(|_| "layout registry lock poisoned".to_string())?;
        by_name.insert(name, entry.clone());
        by_id.insert(component_id, entry);
        Ok(())
    }

    pub fn get_by_name(&self, name: &str) -> Option<LayoutMeta> {
        self.by_name
            .lock()
            .ok()
            .and_then(|m| m.get(name).map(|e| e.layout.clone()))
    }

    pub fn get_by_id(&self, component_id: ComponentId) -> Option<LayoutMeta> {
        self.by_id
            .lock()
            .ok()
            .and_then(|m| m.get(&component_id).map(|e| e.layout.clone()))
    }
}

// ── Encode / decode (DPK1) ─────────────────────────────────────────

pub fn encode(meta: &LayoutMeta) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    write_str(&mut out, &meta.name);
    out.extend_from_slice(&meta.size.to_le_bytes());
    out.extend_from_slice(&meta.align.to_le_bytes());
    let n = meta.fields.len().min(0xffff);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    for f in meta.fields.iter().take(n) {
        out.extend_from_slice(&f.id.to_le_bytes());
        write_str(&mut out, &f.name);
        out.extend_from_slice(&f.offset.to_le_bytes());
        out.extend_from_slice(&f.size.to_le_bytes());
        out.extend_from_slice(&f.align.to_le_bytes());
        out.push(f.wire as u8);
        out.push(f.flags.bits());
        write_opt_str(&mut out, f.doc.as_deref());
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<LayoutMeta, String> {
    if bytes.len() < 6 {
        return Err("layout meta: too short".into());
    }
    if bytes.get(0..4) != Some(MAGIC) {
        return Err("layout meta: bad magic".into());
    }
    if bytes[4] != VERSION {
        return Err(format!("layout meta: unsupported version {}", bytes[4]));
    }
    let mut pos = 5usize;
    let name = read_str(bytes, &mut pos)?;
    let size = read_u32(bytes, &mut pos)?;
    let align = read_u32(bytes, &mut pos)?;
    let n = read_u16(bytes, &mut pos)? as usize;
    let mut fields = Vec::with_capacity(n);
    for _ in 0..n {
        let id = read_u16(bytes, &mut pos)?;
        let fname = read_str(bytes, &mut pos)?;
        let offset = read_u32(bytes, &mut pos)?;
        let fsize = read_u32(bytes, &mut pos)?;
        let falign = read_u32(bytes, &mut pos)?;
        let wire = WireKind::from_u8(read_u8(bytes, &mut pos)?)
            .ok_or_else(|| format!("layout meta: bad wire kind at field {fname}"))?;
        let flags = FieldFlags::from_bits(read_u8(bytes, &mut pos)?)
            .ok_or_else(|| format!("layout meta: bad flags at field {fname}"))?;
        let doc = read_opt_str(bytes, &mut pos)?;
        fields.push(FieldMeta {
            id,
            name: fname,
            offset,
            size: fsize,
            align: falign,
            wire,
            flags,
            doc,
        });
    }
    Ok(LayoutMeta {
        name,
        size,
        align,
        fields,
    })
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(0xffff);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&b[..n]);
}

fn write_opt_str(out: &mut Vec<u8>, s: Option<&str>) {
    match s {
        None => out.push(0),
        Some(t) => {
            out.push(1);
            write_str(out, t);
        }
    }
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, String> {
    let b = *bytes.get(*pos).ok_or("layout meta: unexpected eof")?;
    *pos += 1;
    Ok(b)
}

fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, String> {
    let s = bytes
        .get(*pos..*pos + 2)
        .ok_or("layout meta: unexpected eof")?;
    *pos += 2;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, String> {
    let s = bytes
        .get(*pos..*pos + 4)
        .ok_or("layout meta: unexpected eof")?;
    *pos += 4;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_str(bytes: &[u8], pos: &mut usize) -> Result<String, String> {
    let n = read_u16(bytes, pos)? as usize;
    let s = bytes
        .get(*pos..*pos + n)
        .ok_or("layout meta: unexpected eof")?;
    *pos += n;
    Ok(String::from_utf8_lossy(s).into_owned())
}

fn read_opt_str(bytes: &[u8], pos: &mut usize) -> Result<Option<String>, String> {
    let tag = read_u8(bytes, pos)?;
    match tag {
        0 => Ok(None),
        1 => Ok(Some(read_str(bytes, pos)?)),
        _ => Err("layout meta: bad optional string tag".into()),
    }
}

// ── JSON Schema bridge ───────────────────────────────────────────────

pub fn layout_to_json_schema(meta: &LayoutMeta) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for f in &meta.fields {
        if !f.flags.contains(FieldFlags::INSPECT) && !f.flags.contains(FieldFlags::PREFAB) {
            continue;
        }
        props.insert(f.name.clone(), wire_to_json_schema(f));
        required.push(Value::String(f.name.clone()));
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": meta.name,
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
        "x-deerpunk-layout": {
            "size": meta.size,
            "align": meta.align,
            "format": "DPK1",
        }
    })
}

fn wire_to_json_schema(f: &FieldMeta) -> Value {
    let mut obj = Map::new();
    if let Some(d) = &f.doc {
        obj.insert("description".into(), Value::String(d.clone()));
    }
    obj.insert(
        "x-deerpunk-field".into(),
        json!({
            "id": f.id,
            "offset": f.offset,
            "size": f.size,
            "align": f.align,
            "wire": f.wire as u8,
            "flags": f.flags.bits(),
        }),
    );
    let ty = match f.wire {
        WireKind::Bool => json!({ "type": "boolean" }),
        WireKind::U8 | WireKind::U16 | WireKind::U32 | WireKind::U64 | WireKind::I32 => {
            json!({ "type": "integer" })
        }
        WireKind::F32 => json!({ "type": "number" }),
        WireKind::Vec3 => json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }),
        WireKind::Quat => json!({ "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }),
        WireKind::Indirect => json!({ "type": "string", "description": "indirect / handle" }),
        WireKind::Void => json!({}),
    };
    if let Some(o) = ty.as_object() {
        for (k, v) in o {
            obj.insert(k.clone(), v.clone());
        }
    }
    Value::Object(obj)
}

/// Gamelib/engine provides one [`LayoutMeta`] per `#[repr(C)]` component type.
pub trait LayoutDescribe: 'static {
    fn layout_meta() -> LayoutMeta;
}

pub fn transform_component_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::TransformComponent;

    LayoutMeta {
        name: "TransformComponent".into(),
        size: size_of::<TransformComponent>() as u32,
        align: align_of::<TransformComponent>() as u32,
        fields: vec![
            FieldMeta {
                id: 0,
                name: "position".into(),
                offset: offset_of!(TransformComponent, position) as u32,
                size: 12,
                align: 4,
                wire: WireKind::Vec3,
                flags: FieldFlags::REPLICATE_HOT,
                doc: Some("World position (Z-up)".into()),
            },
            FieldMeta {
                id: 1,
                name: "rotation".into(),
                offset: offset_of!(TransformComponent, rotation) as u32,
                size: 16,
                align: 4,
                wire: WireKind::Quat,
                flags: FieldFlags::REPLICATE_HOT,
                doc: None,
            },
            FieldMeta {
                id: 2,
                name: "scale".into(),
                offset: offset_of!(TransformComponent, scale) as u32,
                size: 12,
                align: 4,
                wire: WireKind::Vec3,
                flags: FieldFlags::REPLICATE_COLD,
                doc: None,
            },
        ],
    }
}

pub fn worldspawn_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::Worldspawn;

    LayoutMeta {
        name: "Worldspawn".into(),
        size: size_of::<Worldspawn>() as u32,
        align: align_of::<Worldspawn>() as u32,
        fields: vec![FieldMeta {
            id: 0,
            name: "map".into(),
            offset: offset_of!(Worldspawn, map) as u32,
            size: 64,
            align: 1,
            wire: WireKind::Indirect,
            flags: FieldFlags::REPLICATE_COLD,
            doc: Some("BSP map name".into()),
        }],
    }
}

pub fn draw_material_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::material::DrawMaterial;

    LayoutMeta {
        name: "DrawMaterial".into(),
        size: size_of::<DrawMaterial>() as u32,
        align: align_of::<DrawMaterial>() as u32,
        fields: vec![FieldMeta {
            id: 0,
            name: "uri".into(),
            offset: offset_of!(DrawMaterial, uri) as u32,
            size: crate::material::component::DRAW_MATERIAL_URI_MAX as u32,
            align: 1,
            wire: WireKind::Indirect,
            flags: FieldFlags::REPLICATE_COLD | FieldFlags::REPLICATE_HOT,
            doc: Some("Material resource URI (server authoritative)".into()),
        }],
    }
}

pub fn mesh3d_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::Mesh3D;

    LayoutMeta {
        name: "Mesh3D".into(),
        size: size_of::<Mesh3D>() as u32,
        align: align_of::<Mesh3D>() as u32,
        fields: vec![FieldMeta {
            id: 0,
            name: "uri".into(),
            offset: offset_of!(Mesh3D, uri) as u32,
            size: crate::components::mesh3d::MESH3D_URI_MAX as u32,
            align: 1,
            wire: WireKind::Indirect,
            flags: FieldFlags::REPLICATE_COLD | FieldFlags::REPLICATE_HOT,
            doc: Some("Mesh resource URI (server authoritative)".into()),
        }],
    }
}

/// Built-in layout for engine `StudioModel`.
pub fn studio_model_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::StudioModel;

    LayoutMeta {
        name: "StudioModel".into(),
        size: size_of::<StudioModel>() as u32,
        align: align_of::<StudioModel>() as u32,
        fields: vec![
            FieldMeta {
                id: 0,
                name: "body".into(),
                offset: offset_of!(StudioModel, body) as u32,
                size: 4,
                align: 4,
                wire: WireKind::U32,
                flags: FieldFlags::PREFAB | FieldFlags::INSPECT | FieldFlags::REPLICATE_HOT,
                doc: Some("Bodygroup variant".into()),
            },
            FieldMeta {
                id: 1,
                name: "skin".into(),
                offset: offset_of!(StudioModel, skin) as u32,
                size: 4,
                align: 4,
                wire: WireKind::U32,
                flags: FieldFlags::PREFAB | FieldFlags::INSPECT | FieldFlags::REPLICATE_HOT,
                doc: Some("Skin index".into()),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_studio_model() {
        let meta = studio_model_layout();
        let bytes = encode(&meta);
        let back = decode(&bytes).expect("decode");
        assert_eq!(back, meta);
        assert!(back.resolve_path("StudioModel.body").is_some());
        assert!(back.resolve_path("body").is_some());
    }

    #[test]
    fn transform_component_layout_offsets() {
        use std::mem::{offset_of, size_of};

        use crate::components::TransformComponent;

        let meta = transform_component_layout();
        assert_eq!(meta.size as usize, size_of::<TransformComponent>());
        let pos = meta.field("position").expect("position");
        let rot = meta.field("rotation").expect("rotation");
        let scale = meta.field("scale").expect("scale");
        assert_eq!(pos.offset, offset_of!(TransformComponent, position) as u32);
        assert_eq!(rot.offset, offset_of!(TransformComponent, rotation) as u32);
        assert_eq!(scale.offset, offset_of!(TransformComponent, scale) as u32);
        // Vec3 (12) + align Quat to 16 → rotation at 16, scale at 32 on typical targets.
        assert_eq!(rot.offset, 16);
        assert_eq!(scale.offset, 32);
    }

    #[test]
    fn json_schema_has_properties() {
        let meta = studio_model_layout();
        let schema = layout_to_json_schema(&meta);
        let props = schema.get("properties").and_then(|v| v.as_object()).unwrap();
        assert!(props.contains_key("body"));
        assert!(props.contains_key("skin"));
    }
}
