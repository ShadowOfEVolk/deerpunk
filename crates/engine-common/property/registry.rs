//! Global property table: path → [`PropertyId`], typed storage, channel flags.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::layout_meta::{FieldFlags, FieldMeta, LayoutMeta, WireKind};
use crate::property::id::PropertyId;
use crate::world::{ComponentId, World};

/// How to read/write this property on an entity.
#[derive(Clone, Debug)]
pub enum PropertyStorage {
    Memory {
        component: String,
        field: String,
        component_id: ComponentId,
        offset: u32,
        wire: WireKind,
    },
    WholeComponent { component: String },
    WorldspawnMap,
    Mesh3dUri,
    DrawMaterialUri,
    MapKvStudioModel,
}

#[derive(Clone, Debug)]
pub struct PropertyDescriptor {
    pub id: PropertyId,
    pub path: String,
    pub flags: FieldFlags,
    pub storage: PropertyStorage,
}

impl PropertyDescriptor {
    pub fn wire_kind(&self) -> WireKind {
        match &self.storage {
            PropertyStorage::Memory { wire, .. } => *wire,
            PropertyStorage::WorldspawnMap
            | PropertyStorage::Mesh3dUri
            | PropertyStorage::DrawMaterialUri
            | PropertyStorage::MapKvStudioModel => WireKind::Indirect,
            PropertyStorage::WholeComponent { .. } => WireKind::Void,
        }
    }

    pub fn component_key(&self) -> &str {
        match &self.storage {
            PropertyStorage::Memory { component, .. } => component.as_str(),
            PropertyStorage::WholeComponent { component } => component.as_str(),
            PropertyStorage::WorldspawnMap => "Worldspawn",
            PropertyStorage::Mesh3dUri => "Mesh3D",
            PropertyStorage::DrawMaterialUri => "DrawMaterial",
            PropertyStorage::MapKvStudioModel => "MapKv",
        }
    }
}

pub struct PropertyRegistry {
    pub(crate) by_id: HashMap<u32, PropertyDescriptor>,
    by_path: HashMap<String, PropertyId>,
    next_id: u32,
}

fn global_registry() -> &'static Mutex<PropertyRegistry> {
    static REG: OnceLock<Mutex<PropertyRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(PropertyRegistry::new()))
}

impl PropertyRegistry {
    pub fn new() -> Self {
        let mut r = Self {
            by_id: HashMap::new(),
            by_path: HashMap::new(),
            next_id: 32,
        };
        r.register_well_known();
        use crate::layout_meta::{studio_model_layout, transform_component_layout, worldspawn_layout};
        let placeholder: ComponentId = 0;
        for layout in [
            transform_component_layout(),
            worldspawn_layout(),
            studio_model_layout(),
        ] {
            r.register_layout(placeholder, &layout);
        }
        r
    }

    pub fn global() -> &'static Mutex<PropertyRegistry> {
        global_registry()
    }

    pub fn resolve_path(&self, path: &str) -> Option<PropertyId> {
        let path = canonical_property_path(path);
        self.by_path.get(&path).copied()
    }

    pub fn get(&self, id: PropertyId) -> Option<&PropertyDescriptor> {
        self.by_id.get(&id.0)
    }

    pub fn wire_kind(&self, id: PropertyId) -> Option<WireKind> {
        self.get(id).map(|d| d.wire_kind())
    }

    pub fn component_key(&self, id: PropertyId) -> Option<&str> {
        self.get(id).map(|d| d.component_key())
    }

    /// DPK1 replication + all other layout-backed properties (save, prefab, …).
    pub fn register_layout(&mut self, component_id: ComponentId, layout: &LayoutMeta) {
        for field in &layout.fields {
            if !field.flags.intersects(
                FieldFlags::PREFAB
                    | FieldFlags::INSPECT
                    | FieldFlags::REPLICATE_HOT
                    | FieldFlags::REPLICATE_COLD
                    | FieldFlags::SAVE
                    | FieldFlags::SCRIPT,
            ) {
                continue;
            }
            let path = format!("{}.{}", layout.name, field.name);
            self.upsert_layout_field(component_id, layout, field, &path);
        }
    }

    fn upsert_layout_field(
        &mut self,
        component_id: ComponentId,
        layout: &LayoutMeta,
        field: &FieldMeta,
        path: &str,
    ) {
        let path = canonical_property_path(path);
        let id = self.id_for_path(&path);
        let storage = match path.as_str() {
            "Worldspawn.map" => PropertyStorage::WorldspawnMap,
            "Mesh3D.uri" => PropertyStorage::Mesh3dUri,
            "DrawMaterial.uri" => PropertyStorage::DrawMaterialUri,
            _ => PropertyStorage::Memory {
                component: layout.name.clone(),
                field: field.name.clone(),
                component_id,
                offset: field.offset,
                wire: field.wire,
            },
        };

        let desc = PropertyDescriptor {
            id,
            path: path.clone(),
            flags: field.flags,
            storage,
        };
        self.by_id.insert(id.0, desc);
        self.by_path.insert(path, id);
    }

    fn id_for_path(&mut self, path: &str) -> PropertyId {
        if let Some(id) = self.by_path.get(path) {
            return *id;
        }
        match path {
            "TransformComponent.position" => PropertyId::TRANSFORM_POSITION,
            "TransformComponent.rotation" => PropertyId::TRANSFORM_ROTATION,
            "TransformComponent.scale" => PropertyId::TRANSFORM_SCALE,
            "Worldspawn.map" => PropertyId::WORLDSPAWN_MAP,
            "Mesh3D.uri" => PropertyId::MESH3D_MESH,
            "DrawMaterial.uri" => PropertyId::DRAW_MATERIAL_URI,
            "StudioModel.body" => PropertyId::STUDIO_MODEL_BODY,
            "StudioModel.skin" => PropertyId::STUDIO_MODEL_SKIN,
            _ => {
                let id = PropertyId(self.next_id);
                self.next_id += 1;
                id
            }
        }
    }

    fn register_well_known(&mut self) {
        let wholes = [
            (PropertyId::CAMERA_COMPONENT, "CameraComponent"),
            (PropertyId::CHARACTER_CONTROLLER, "CharacterController"),
            (PropertyId::SOUND_EMITTER, "SoundEmitter"),
            (PropertyId::MATERIAL, "Material"),
            (PropertyId::WORLDSPAWN, "Worldspawn"),
            (PropertyId::MESH3D, "Mesh3D"),
        ];
        for (id, name) in wholes {
            self.insert_whole(id, name, FieldFlags::REPLICATE_COLD);
        }
        self.insert_special(
            PropertyId::MAPKV_STUDIO_MODEL,
            "MapKv.model",
            PropertyStorage::MapKvStudioModel,
            FieldFlags::REPLICATE_COLD,
        );
    }

    fn insert_whole(&mut self, id: PropertyId, component: &str, flags: FieldFlags) {
        let path = component.to_string();
        let desc = PropertyDescriptor {
            id,
            path: path.clone(),
            flags,
            storage: PropertyStorage::WholeComponent {
                component: component.into(),
            },
        };
        self.by_id.insert(id.0, desc);
        self.by_path.insert(path, id);
    }

    fn insert_special(
        &mut self,
        id: PropertyId,
        path: &str,
        storage: PropertyStorage,
        flags: FieldFlags,
    ) {
        let path = canonical_property_path(path);
        let desc = PropertyDescriptor {
            id,
            path: path.clone(),
            flags,
            storage,
        };
        self.by_id.insert(id.0, desc);
        self.by_path.insert(path, id);
    }
}

fn canonical_property_path(path: &str) -> String {
    let path = path.trim();
    match path {
        "Mesh3D.mesh" => "Mesh3D.uri".to_string(),
        "DrawMaterial.material" => "DrawMaterial.uri".to_string(),
        other => other.to_string(),
    }
}

/// Register gamelib/engine layout fields (replicate + save + prefab + …).
pub fn sync_properties_from_layout(component_id: ComponentId, layout: &LayoutMeta) {
    let mut reg = global_registry()
        .lock()
        .expect("property registry lock poisoned");
    reg.register_layout(component_id, layout);
}

/// Engine builtins at world init.
pub fn register_engine_layout_properties(world: &World) {
    use crate::layout_meta::{
        draw_material_layout, mesh3d_layout, transform_component_layout, worldspawn_layout,
    };

    let t_id = world.component_init::<crate::components::TransformComponent>();
    let layout = transform_component_layout();
    let _ = world.layout_registry().register(t_id, layout.clone());
    sync_properties_from_layout(t_id, &layout);

    let ws_id = world.component_init::<crate::components::Worldspawn>();
    let layout = worldspawn_layout();
    let _ = world.layout_registry().register(ws_id, layout.clone());
    sync_properties_from_layout(ws_id, &layout);

    let mesh_id = world.component_init::<crate::components::Mesh3D>();
    let layout = mesh3d_layout();
    let _ = world.layout_registry().register(mesh_id, layout.clone());
    sync_properties_from_layout(mesh_id, &layout);

    let mat_id = world.component_init::<DrawMaterial>();
    let layout = draw_material_layout();
    let _ = world.layout_registry().register(mat_id, layout.clone());
    sync_properties_from_layout(mat_id, &layout);
}

use crate::material::DrawMaterial;
