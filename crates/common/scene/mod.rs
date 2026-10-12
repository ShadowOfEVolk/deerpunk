//! Prefabs: `{gamedir}/prefabs/*.prefab` (jsonc).
//! Map entities: `classname` → `{gamedir}/prefabs/{classname}.prefab`.

mod apply;
mod engine_inspect;
mod prefab_appliers;
mod entity_def;
mod map_submodel;
mod format;
mod load;
mod map_light;
mod map_spawn;
mod prefab_manager;
mod spawner;

pub use apply::{apply_component, apply_prefab_component_value};
pub use map_spawn::{apply_map_spawn_tail, MapSpawnOpts};
pub use engine_inspect::register_engine_component_inspect;
pub use prefab_appliers::register_engine_prefab_appliers;
pub use entity_def::EntityDef;
pub use prefab_manager::PrefabManager;
pub use load::{
    SceneLoadError, load_prefab_file, spawn_prefab, spawn_prefab_entity,
    spawn_prefab_filtered, spawn_prefab_filtered_with_overrides,
    spawn_prefab_filtered_with_overrides_cached,
};
pub use map_submodel::BspSubmodel;
pub use spawner::{EntitySpawnDescriptor, EntitySpawner};
