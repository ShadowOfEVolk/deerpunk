//! `inventory` collection of `#[game_component]` registrars (linked into gamelib DLL).

use crate::{ComponentId, EngineApi};

inventory::collect!(GameComponentRegistrar);

/// Which world(s) a `#[game_component]` type targets (`client` / default server / `both`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameComponentWorlds {
    Server,
    Client,
    Both,
}

/// Submitted by `#[game_component]` from each gamelib crate in the same `.so`.
pub struct GameComponentRegistrar {
    pub name: &'static str,
    pub register: fn(&EngineApi) -> ComponentId,
    pub worlds: GameComponentWorlds,
}

/// Register every submitted component (safe to call more than once if `register` is idempotent).
pub fn register_inventory_components(api: &EngineApi) {
    for entry in inventory::iter::<GameComponentRegistrar> {
        (entry.register)(api);
    }
}

/// Register server + `both` components (typical server gamelib `init_server_components`).
pub fn register_server_inventory_components(api: &EngineApi) {
    for entry in inventory::iter::<GameComponentRegistrar> {
        if matches!(
            entry.worlds,
            GameComponentWorlds::Server | GameComponentWorlds::Both
        ) {
            (entry.register)(api);
        }
    }
}

/// Register client + `both` components (typical client gamelib init).
pub fn register_client_inventory_components(api: &EngineApi) {
    for entry in inventory::iter::<GameComponentRegistrar> {
        if matches!(
            entry.worlds,
            GameComponentWorlds::Client | GameComponentWorlds::Both
        ) {
            (entry.register)(api);
        }
    }
}