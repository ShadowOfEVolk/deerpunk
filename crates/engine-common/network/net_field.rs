//! Network field registry — wire ids and layout-backed read/apply metadata (DPK1 consumer).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::layout_meta::{FieldFlags, FieldMeta, LayoutMeta, WireKind};
use crate::world::{ComponentId, World};

/// Stable wire id for one replicated prefab path (`TransformComponent.position`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NetFieldId(pub u16);

/// Back-compat alias while call sites migrate naming.
pub type FieldPathId = NetFieldId;

impl NetFieldId {
    pub const TRANSFORM_POSITION: Self = Self(1);
    pub const TRANSFORM_ROTATION: Self = Self(2);
    pub const TRANSFORM_SCALE: Self = Self(3);
    pub const CAMERA_COMPONENT: Self = Self(10);
    pub const CHARACTER_CONTROLLER: Self = Self(11);
    pub const SOUND_EMITTER: Self = Self(12);
    pub const MATERIAL: Self = Self(13);
    pub const WORLDSPAWN: Self = Self(14);
    pub const MESH3D: Self = Self(15);
    pub const WORLDSPAWN_MAP: Self = Self(16);
    pub const MESH3D_MESH: Self = Self(18);
    pub const STUDIO_MODEL_BODY: Self = Self(19);
    pub const STUDIO_MODEL_SKIN: Self = Self(20);
    /// GoldSrc / Hammer `model` key on [`MapKv`] (`.mdl` path for [`StudioModel`]).
    pub const MAPKV_STUDIO_MODEL: Self = Self(21);
    pub const DRAW_MATERIAL_URI: Self = Self(22);

    pub fn from_u16(v: u16) -> Option<Self> {
        global_registry()
            .lock()
            .ok()
            .and_then(|r| r.get(Self(v)).map(|_| Self(v)))
    }

    #[deprecated(note = "wire ids are u16; use from_u16")]
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::from_u16(v as u16)
    }

    pub fn as_u16(self) -> u16 {
        self.0
    }

    #[deprecated(note = "wire ids are u16; use as_u16")]
    pub fn as_u8(self) -> u8 {
        self.0 as u8
    }

    /// Resolve prefab path using the process-wide field registry (client + server share registration order).
    pub fn parse(path: &str) -> Option<Self> {
        global_registry()
            .lock()
            .ok()
            .and_then(|r| r.resolve_path(path))
    }
}

#[derive(Clone, Debug)]
pub enum NetFieldKind {
    /// `base + offset`, encode/decode via [`WireKind`].
    Memory {
        component: String,
        field: String,
        component_id: ComponentId,
        offset: u32,
        wire: WireKind,
    },
    /// Cold marker: client applies whole component from local prefab.
    WholeComponent { component: String },
    /// `Worldspawn.map` (fixed buffer helpers).
    WorldspawnMap,
    /// [`Mesh3D::uri`] (length-tagged string on the wire).
    Mesh3dUri,
    /// [`DrawMaterial::uri`] (length-tagged string on the wire).
    DrawMaterialUri,
    /// `MapKv` key `model` (studio `.mdl` path).
    MapKvStudioModel,
}

#[derive(Clone, Debug)]
pub struct NetFieldDescriptor {
    pub id: NetFieldId,
    pub path: String,
    pub kind: NetFieldKind,
    pub flags: FieldFlags,
}

pub struct NetFieldRegistry {
    by_id: HashMap<u16, NetFieldDescriptor>,
    by_path: HashMap<String, NetFieldId>,
    next_auto: u16,
}

fn global_registry() -> &'static Mutex<NetFieldRegistry> {
    static REG: OnceLock<Mutex<NetFieldRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(NetFieldRegistry::new()))
}

impl NetFieldRegistry {
    pub fn new() -> Self {
        let mut r = Self {
            by_id: HashMap::new(),
            by_path: HashMap::new(),
            next_auto: 32,
        };
        r.register_well_known();
        use crate::layout_meta::{studio_model_layout, transform_component_layout, worldspawn_layout};
        let placeholder: ComponentId = 0;
        for layout in [
            transform_component_layout(),
            worldspawn_layout(),
            studio_model_layout(),
        ] {
            r.merge_layout(placeholder, &layout);
        }
        r
    }

    /// Apply DPK1 replication fields (updates component ids when re-registered from a live [`World`]).
    pub fn merge_layout(&mut self, component_id: ComponentId, layout: &LayoutMeta) {
        for field in &layout.fields {
            if !field
                .flags
                .intersects(FieldFlags::REPLICATE_HOT | FieldFlags::REPLICATE_COLD)
            {
                continue;
            }
            let path = format!("{}.{}", layout.name, field.name);
            let id = match path.as_str() {
                "TransformComponent.position" => NetFieldId::TRANSFORM_POSITION,
                "TransformComponent.rotation" => NetFieldId::TRANSFORM_ROTATION,
                "TransformComponent.scale" => NetFieldId::TRANSFORM_SCALE,
                "Worldspawn.map" => NetFieldId::WORLDSPAWN_MAP,
                "Mesh3D.uri" => NetFieldId::MESH3D_MESH,
                "DrawMaterial.uri" => NetFieldId::DRAW_MATERIAL_URI,
                "StudioModel.body" => NetFieldId::STUDIO_MODEL_BODY,
                "StudioModel.skin" => NetFieldId::STUDIO_MODEL_SKIN,
                _ => self
                    .resolve_path(&path)
                    .unwrap_or_else(|| {
                        let id = NetFieldId(self.next_auto);
                        self.next_auto += 1;
                        id
                    }),
            };
            if path == "Worldspawn.map" {
                self.upsert_special(id, &path, NetFieldKind::WorldspawnMap, field.flags);
            } else if path == "Mesh3D.uri" {
                self.upsert_special(id, &path, NetFieldKind::Mesh3dUri, field.flags);
            } else if path == "DrawMaterial.uri" {
                self.upsert_special(id, &path, NetFieldKind::DrawMaterialUri, field.flags);
            } else {
                self.upsert_memory(id, &path, component_id, layout, field);
            }
        }
    }

    pub fn global() -> &'static Mutex<NetFieldRegistry> {
        global_registry()
    }

    pub fn resolve_path(&self, path: &str) -> Option<NetFieldId> {
        let path = canonical_path(path);
        self.by_path.get(&path).copied()
    }

    pub fn get(&self, id: NetFieldId) -> Option<&NetFieldDescriptor> {
        self.by_id.get(&id.0)
    }

    pub fn wire_kind(&self, id: NetFieldId) -> Option<WireKind> {
        let d = self.get(id)?;
        match &d.kind {
            NetFieldKind::Memory { wire, .. } => Some(*wire),
            NetFieldKind::WorldspawnMap
            | NetFieldKind::Mesh3dUri
            | NetFieldKind::DrawMaterialUri
            | NetFieldKind::MapKvStudioModel => Some(WireKind::Indirect),
            NetFieldKind::WholeComponent { .. } => Some(WireKind::Void),
        }
    }

    pub fn component_key(&self, id: NetFieldId) -> Option<&str> {
        let d = self.get(id)?;
        match &d.kind {
            NetFieldKind::Memory { component, .. } => Some(component.as_str()),
            NetFieldKind::WholeComponent { component } => Some(component.as_str()),
            NetFieldKind::WorldspawnMap => Some("Worldspawn"),
            NetFieldKind::Mesh3dUri => Some("Mesh3D"),
            NetFieldKind::DrawMaterialUri => Some("DrawMaterial"),
            NetFieldKind::MapKvStudioModel => Some("MapKv"),
        }
    }

    pub fn ingest_layout(&mut self, component_id: ComponentId, layout: &LayoutMeta) {
        for field in &layout.fields {
            if !field
                .flags
                .intersects(FieldFlags::REPLICATE_HOT | FieldFlags::REPLICATE_COLD)
            {
                continue;
            }
            let path = format!("{}.{}", layout.name, field.name);
            if self.by_path.contains_key(&path) {
                continue;
            }
            let id = self.wire_id_for_layout_field(&layout.name, &field.name);
            self.insert(
                id,
                path,
                NetFieldKind::Memory {
                    component: layout.name.clone(),
                    field: field.name.clone(),
                    component_id,
                    offset: field.offset,
                    wire: field.wire,
                },
                field.flags,
            );
        }
    }

    fn wire_id_for_layout_field(&mut self, component: &str, field: &str) -> NetFieldId {
        let path = format!("{}.{}", component, field);
        match path.as_str() {
            "TransformComponent.position" => NetFieldId::TRANSFORM_POSITION,
            "TransformComponent.rotation" => NetFieldId::TRANSFORM_ROTATION,
            "TransformComponent.scale" => NetFieldId::TRANSFORM_SCALE,
            "Worldspawn.map" => NetFieldId::WORLDSPAWN_MAP,
            "StudioModel.body" => NetFieldId::STUDIO_MODEL_BODY,
            "StudioModel.skin" => NetFieldId::STUDIO_MODEL_SKIN,
            _ => {
                let id = NetFieldId(self.next_auto);
                self.next_auto += 1;
                id
            }
        }
    }

    fn register_well_known(&mut self) {
        let wholes = [
            (NetFieldId::CAMERA_COMPONENT, "CameraComponent"),
            (NetFieldId::CHARACTER_CONTROLLER, "CharacterController"),
            (NetFieldId::SOUND_EMITTER, "SoundEmitter"),
            (NetFieldId::MATERIAL, "Material"),
            (NetFieldId::WORLDSPAWN, "Worldspawn"),
            (NetFieldId::MESH3D, "Mesh3D"),
        ];
        for (id, name) in wholes {
            self.insert(
                id,
                name.into(),
                NetFieldKind::WholeComponent {
                    component: name.into(),
                },
                FieldFlags::REPLICATE_COLD,
            );
        }
        self.insert(
            NetFieldId::MAPKV_STUDIO_MODEL,
            "MapKv.model".into(),
            NetFieldKind::MapKvStudioModel,
            FieldFlags::REPLICATE_COLD,
        );
    }

    fn insert(&mut self, id: NetFieldId, path: String, kind: NetFieldKind, flags: FieldFlags) {
        let path = canonical_path(&path);
        let desc = NetFieldDescriptor {
            id,
            path: path.clone(),
            kind,
            flags,
        };
        self.by_id.insert(id.0, desc);
        self.by_path.insert(path, id);
    }

    fn upsert_special(
        &mut self,
        id: NetFieldId,
        path: &str,
        kind: NetFieldKind,
        flags: FieldFlags,
    ) {
        let path = canonical_path(path);
        let desc = NetFieldDescriptor {
            id,
            path: path.clone(),
            kind,
            flags,
        };
        self.by_id.insert(id.0, desc);
        self.by_path.insert(path, id);
    }

    fn upsert_memory(
        &mut self,
        id: NetFieldId,
        path: &str,
        component_id: ComponentId,
        layout: &LayoutMeta,
        field: &FieldMeta,
    ) {
        let path = canonical_path(path);
        let kind = NetFieldKind::Memory {
            component: layout.name.clone(),
            field: field.name.clone(),
            component_id,
            offset: field.offset,
            wire: field.wire,
        };
        let desc = NetFieldDescriptor {
            id,
            path: path.clone(),
            kind,
            flags: field.flags,
        };
        self.by_id.insert(id.0, desc);
        self.by_path.insert(path, id);
    }
}

fn canonical_path(path: &str) -> String {
    let path = path.trim();
    match path {
        "Mesh3D.mesh" => "Mesh3D.uri".to_string(),
        "DrawMaterial.material" => "DrawMaterial.uri".to_string(),
        other => other.to_string(),
    }
}

/// Register engine + gamelib layouts into the global net field table.
pub fn sync_net_fields_from_layout(component_id: ComponentId, layout: &LayoutMeta) {
    let mut reg = global_registry()
        .lock()
        .expect("net field registry lock poisoned");
    reg.merge_layout(component_id, layout);
}

/// Engine builtins: transform + worldspawn layouts (StudioModel from prefab_appliers).
pub fn register_engine_layout_net_fields(world: &World) {
    use crate::layout_meta::{
        draw_material_layout, mesh3d_layout, transform_component_layout, worldspawn_layout,
    };

    let t_id = world.component_init::<crate::components::TransformComponent>();
    let layout = transform_component_layout();
    let _ = world.layout_registry().register(t_id, layout.clone());
    sync_net_fields_from_layout(t_id, &layout);

    let ws_id = world.component_init::<crate::components::Worldspawn>();
    let layout = worldspawn_layout();
    let _ = world.layout_registry().register(ws_id, layout.clone());
    sync_net_fields_from_layout(ws_id, &layout);

    let mesh_id = world.component_init::<crate::components::Mesh3D>();
    let layout = mesh3d_layout();
    let _ = world.layout_registry().register(mesh_id, layout.clone());
    sync_net_fields_from_layout(mesh_id, &layout);

    let dm_id = world.component_init::<crate::material::DrawMaterial>();
    let layout = draw_material_layout();
    let _ = world.layout_registry().register(dm_id, layout.clone());
    sync_net_fields_from_layout(dm_id, &layout);
}

impl World {
    pub fn net_field_registry(&self) -> &'static Mutex<NetFieldRegistry> {
        NetFieldRegistry::global()
    }
}
