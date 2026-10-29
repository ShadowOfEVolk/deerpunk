pub mod console_cmd;
pub mod cvar_def;
pub mod cvar;
pub mod atlas;
pub mod lightmap_atlas;
pub mod bsp_entities;
pub mod components;
pub mod input;
pub mod logger;
pub mod network;
pub mod goldsrc_lightstyle;
pub mod component_inspect;
pub mod layout_meta;
pub mod prefab;
pub mod scene;
pub mod wad {
    pub use deerpunk_engine_resource::wad::*;
}
pub mod world;
pub mod ecs_validate;

pub mod coord;
pub mod events;
pub mod frame;
pub mod material;
pub mod physics;
pub use deerpunk_engine_resource as resource;
pub mod sound_event;
pub mod scheduler;
pub mod transform;
pub mod transform_hierarchy;
pub mod ui_draw;

pub use components::{
    CameraComponent, LightmapId, MapKv, MeshDrawAux, MeshRenderer, MeshTexture,
    NetReplicatedComponent, NetworkEntity, PendingAlbedoAtlas, PendingLightmap,
    PointLight,
    SimpleEntity, Terrain, TerrainCollider, TerrainNeedRebuild, TerrainRenderer,
    TerrainSplatMaterial, TransformComponent, UiDocumentComponent, Worldspawn,
    NET_FIELDS_MAX, NET_PREFAB_MAX,
};
pub use goldsrc_lightstyle::{LightstyleTable, MAX_LIGHTSTYLES};
pub use network::replication::{NetworkComponent, NetworkField};

pub use lightmap_atlas::{LightmapAtlasLayers, LM_STYLE_SLOTS};
pub use transform::Transform;
pub mod util;

pub use layout_meta::{
    decode as decode_layout_meta, encode as encode_layout_meta, layout_to_json_schema,
    draw_material_layout, mesh3d_layout, transform_component_layout, worldspawn_layout, FieldFlags,
    FieldMeta, FieldId,
    LayoutDescribe,
    LayoutMeta, LayoutMetaRegistry, WireKind,
};
pub use material::{
    DrawMaterial, GpuMaterialIndex, MaterialAsset, MaterialHandle, PropertyValueSer,
    SceneLighting, ShaderError, ShaderId, ShaderMeta, ShaderPreprocessor,
};
