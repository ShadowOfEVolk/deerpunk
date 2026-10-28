//! Prefabs: `{gamedir}/prefabs/*.prefab` (jsonc).
//! Map entities: `classname` → `{gamedir}/prefabs/{classname}.prefab`.

mod apply;
mod draw_material_uri;
mod engine_inspect;
mod entity_def;
mod format;
mod load;
mod map_light;
mod map_spawn;
mod map_submodel;
mod mesh_attach;
mod mesh_uri;
mod prefab_appliers;
mod prefab_manager;
mod spawner;
mod studio_present;

pub use apply::{apply_component, apply_prefab_component_value};
pub use draw_material_uri::{draw_material_uri_of, set_draw_material_uri};
pub use engine_inspect::register_engine_component_inspect;
pub use entity_def::EntityDef;
pub use load::{
    load_prefab_file, spawn_prefab, spawn_prefab_entity, spawn_prefab_filtered,
    spawn_prefab_filtered_with_component_overrides_cached, spawn_prefab_filtered_with_overrides,
    spawn_prefab_filtered_with_overrides_cached, SceneLoadError,
};
pub use map_spawn::{apply_map_spawn_tail, MapSpawnOpts};
pub use map_submodel::BspSubmodel;
pub use mesh_attach::{attach_mesh3d, attach_skinned, release_mesh3d_at, release_skinned_at};
pub use mesh_uri::{apply_mesh3d_uri, mesh3d_uri_of, resolve_mesh3d_uri, set_mesh3d_uri};
pub use prefab_appliers::register_engine_prefab_appliers;
pub use prefab_manager::PrefabManager;
pub use spawner::{EntitySpawnDescriptor, EntitySpawner};
pub use studio_present::{register_studio_present_observers, sync_studio_presentation};
