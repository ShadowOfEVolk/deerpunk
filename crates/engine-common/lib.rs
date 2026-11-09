pub mod atlas;
pub mod bsp_entities;
pub mod component_inspect;
pub mod components;
pub mod console_cmd;
pub mod cvar;
pub mod cvar_def;
pub mod input;
pub mod layout_meta;
pub mod logger;
pub mod network;
pub mod prefab;
pub mod prefab_layout_apply;
pub mod property;
pub mod scene;
pub mod wad {
    pub use deerpunk_engine_resource::wad::*;
}
pub mod ecs_observer;
pub mod ecs_validate;
pub mod world;

pub mod coord;
pub mod events;
pub mod frame;
pub mod loader_queue;
pub mod material;
pub mod mesh_submit;
pub mod physics;
pub use deerpunk_engine_resource as resource;
pub mod scheduler;
pub mod sound_event;
pub mod transform;
pub mod transform_hierarchy;
pub mod ui_draw;

pub use components::{
    CameraComponent, MapKv, MeshRenderer, MeshTexture, NetReplicatedComponent, NetworkEntity,
    PendingAlbedoAtlas, PointLight, SimpleEntity, Terrain, TerrainCollider, TerrainNeedRebuild,
    TerrainRenderer, TerrainSplatMaterial, TransformComponent, UiDocumentComponent, Worldspawn,
    NET_FIELDS_MAX, NET_PREFAB_MAX,
};
pub use network::replication::{NetworkComponent, NetworkField};

pub use transform::Transform;
pub mod util;

pub use layout_meta::{
    decode as decode_layout_meta, draw_material_layout, encode as encode_layout_meta,
    layout_to_json_schema, mesh3d_layout, transform_component_layout, worldspawn_layout,
    ComponentRole, FieldFlags, FieldId, FieldMeta, LayoutDescribe, LayoutMeta, LayoutMetaRegistry,
    WireKind,
};
pub use material::{
    DrawMaterial, GpuMaterialIndex, MaterialAsset, MaterialHandle, PropertyValueSer, SceneLighting,
    ShaderError, ShaderId, ShaderMeta, ShaderPreprocessor, VertexLayoutSpec,
};
pub use property::{
    read as property_read, read_by_path as property_read_by_path,
    register_engine_layout_properties, snapshot_component_save, sync_properties_from_layout,
    write as property_write, write_by_path as property_write_by_path, PropertyChannel,
    PropertyDescriptor, PropertyError, PropertyId, PropertyRegistry, PropertyStorage, PropertyType,
    PropertyValue,
};

/// Property registry follows component layout blobs registered on [`world::World`].
pub fn install_property_layout_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        deerpunk_engine_world::set_layout_hook(crate::property::sync_properties_from_layout);
    });
}
