//! Server-owned scenes: `{gamedir}/scenes/*.scene` + `{gamedir}/prefabs/*.prefab` (jsonc).
//! Map entities: `classname` → `{gamedir}/prefabs/{classname}.prefab`.

mod apply;
mod bsp_place;
mod entity_def;
mod format;
mod load;
mod map_light;
mod prefab_manager;
mod spawner;

pub use bsp_place::place_bsp_map;
pub use load::{SceneLoadError, load_prefab_file, load_scene, spawn_prefab};
pub use spawner::{spawn_map_entities, EntitySpawnDescriptor, EntitySpawner};
