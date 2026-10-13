pub mod atlas;
pub mod lightmap_atlas;
pub mod bsp_entities;
pub mod components;
pub mod input;
pub mod ipc;
pub mod logger;
pub mod network;
pub mod goldsrc_lightstyle;
pub mod component_inspect;
pub mod prefab;
pub mod scene;
pub mod wad;
pub mod world;

pub mod coord;
pub mod events;
pub mod frame;
pub mod frame_buffer;
pub mod material;
pub mod physics;
pub mod resource;
pub mod scheduler;
pub mod transform;
pub mod ui_draw;

pub use components::{
    CameraComponent, LightmapId, MapKv, MeshRenderer, MeshTexture,
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

pub use material::{
    DrawMaterial, GpuMaterialIndex, MaterialAsset, MaterialHandle, PropertyValueSer,
    SceneLighting, ShaderError, ShaderId, ShaderMeta, ShaderPreprocessor,
};
