//! `inventory` collection of `#[game_component]` registrars (linked into gamelib DLL).

use crate::{ComponentId, EngineApi};

inventory::collect!(GameComponentRegistrar);

/// Submitted by `#[game_component]` from each gamelib crate in the same `.so`.
pub struct GameComponentRegistrar {
    pub name: &'static str,
    pub register: fn(&EngineApi) -> ComponentId,
}

/// Register every submitted component (safe to call more than once if `register` is idempotent).
pub fn register_inventory_components(api: &EngineApi) {
    for entry in inventory::iter::<GameComponentRegistrar> {
        (entry.register)(api);
    }
}
