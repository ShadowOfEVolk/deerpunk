//! Shared scenes: `{gamedir}/scenes/*.scene` + `{gamedir}/prefabs/*.prefab` (jsonc).
//! Map entities: `classname` → `{gamedir}/prefabs/{classname}.prefab`.

mod apply;
mod entity_def;
mod map_submodel;
mod format;
mod load;
mod map_light;
mod prefab_manager;
mod spawner;

pub use entity_def::EntityDef;
pub use prefab_manager::PrefabManager;
pub use load::{
    SceneLoadError, load_prefab_file, load_scene, spawn_prefab, spawn_prefab_filtered,
    spawn_prefab_filtered_with_overrides, spawn_prefab_filtered_with_overrides_cached,
};
pub use map_submodel::BspSubmodel;
pub use spawner::{EntitySpawnDescriptor, EntitySpawner};
