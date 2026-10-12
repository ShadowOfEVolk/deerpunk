//! Client-side prefab replication: apply SpawnPrefab / NetDelta / NetDestroy.

use std::collections::HashMap;
use std::path::Path;

use deerpunk_engine_common::components::NetworkEntity;
use deerpunk_engine_common::network::field_apply::apply_field;
use deerpunk_engine_common::network::field_path::{FieldPathId, FieldValue};
use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::{
    NetDeltaRpc, NetDestroyRpc, PrefabDefRpc, SpawnPrefabRpc,
};
use deerpunk_engine_common::scene::PrefabManager;
use deerpunk_engine_common::scene::spawn_prefab_filtered_with_overrides_cached;
use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::{EntityId, World};

#[derive(Default)]
pub struct ClientPrefabReplication {
    /// Server-pushed prefab defs (authoritative on remote connect).
    network_prefabs: PrefabManager,
    /// net_id → client entity
    map: HashMap<u16, EntityId>,
    /// Static (net_id=0) spawns — destroyed on clear.
    static_entities: Vec<EntityId>,
    /// Server-authored view target (`UserMessage SetView`).
    view_net_id: Option<u16>,
}

impl ClientPrefabReplication {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self, world: &World) {
        for &id in self.map.values() {
            world.destroy(id);
        }
        for &id in &self.static_entities {
            world.destroy(id);
        }
        self.map.clear();
        self.static_entities.clear();
        self.view_net_id = None;
        self.network_prefabs.clear_cache();
        world.simulate(0.0);
    }

    pub fn client_of_net(&self, net_id: u16) -> Option<EntityId> {
        self.map.get(&net_id).copied()
    }

    pub fn view_net_id(&self) -> Option<u16> {
        self.view_net_id
    }

    pub fn view_entity(&self) -> Option<EntityId> {
        self.view_net_id.and_then(|id| self.client_of_net(id))
    }

    pub fn set_view_net_id(&mut self, net_id: u16) {
        self.view_net_id = if net_id == 0 { None } else { Some(net_id) };
        log::info!("[prefab_rep] SetView net_id={net_id}");
    }

    /// World geometry from join `Map` UM (cleared with [`Self::clear`]).
    pub fn track_static_entity(&mut self, entity: EntityId) {
        self.static_entities.push(entity);
    }

    /// Apply one s2c packet (SVC byte + payload). Returns true if handled.
    pub fn apply_packet(
        &mut self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        packet: &[u8],
    ) -> bool {
        let Some((&cmd, payload)) = packet.split_first() else {
            return false;
        };
        if cmd == SVC::SpawnPrefab as u8 {
            match SpawnPrefabRpc::decode(payload) {
                Some(rpc) => {
                    if let Err(e) = self.apply_spawn(world, gamedir, resources, rpc) {
                        log::error!("[prefab_rep] spawn: {e}");
                    }
                    true
                }
                None => {
                    log::warn!("[prefab_rep] bad SpawnPrefab");
                    true
                }
            }
        } else if cmd == SVC::NetDelta as u8 {
            match NetDeltaRpc::decode(payload) {
                Some(rpc) => {
                    self.apply_delta(world, rpc);
                    true
                }
                None => {
                    log::warn!("[prefab_rep] bad NetDelta");
                    true
                }
            }
        } else if cmd == SVC::NetDestroy as u8 {
            match NetDestroyRpc::decode(payload) {
                Some(rpc) => {
                    self.apply_destroy(world, rpc);
                    true
                }
                None => {
                    log::warn!("[prefab_rep] bad NetDestroy");
                    true
                }
            }
        } else if cmd == SVC::PrefabDef as u8 {
            match PrefabDefRpc::decode(payload) {
                Some(rpc) => {
                    self.network_prefabs.insert_cached(&rpc.name, rpc.def);
                    log::info!("[prefab_rep] PrefabDef '{}'", rpc.name);
                    true
                }
                None => {
                    log::warn!("[prefab_rep] bad PrefabDef");
                    true
                }
            }
        } else if cmd == SVC::UserMessage as u8 {
            // Handled in host `parse_server_message` → `um` hooks.
            false
        } else {
            false
        }
    }

    fn apply_spawn(
        &mut self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        rpc: SpawnPrefabRpc,
    ) -> Result<(), String> {
        if rpc.net_id != 0 {
            if let Some(old) = self.map.remove(&rpc.net_id) {
                world.destroy(old);
            }
        }

        let mut allow_keys: Vec<&str> = vec!["TransformComponent"];
        for (path, _) in &rpc.cold {
            if let Some(k) = path.component_key() {
                if !allow_keys.contains(&k) {
                    allow_keys.push(k);
                }
            }
        }
        // NetworkEntity itself on client (for debugging / net_id stamp)
        if rpc.net_id != 0 {
            allow_keys.push("NetworkEntity");
        }
        // worldspawn also needs Material (in cold) — Mesh3D comes from Worldspawn apply
        if rpc.prefab == "worldspawn" {
            for k in ["Worldspawn", "Material", "TransformComponent"] {
                if !allow_keys.contains(&k) {
                    allow_keys.push(k);
                }
            }
        }

        let allow: Vec<&str> = allow_keys;
        let entity = spawn_prefab_filtered_with_overrides_cached(
            world,
            gamedir,
            resources,
            &rpc.prefab,
            Some(&allow),
            &rpc.cold,
            Some(&self.network_prefabs),
        )
        .map_err(|e| e.message)?;

        if rpc.net_id != 0 {
            let ne_id = world.component_init::<NetworkEntity>();
            if world.has(entity, ne_id) {
                let ptr = world.get_raw(entity, ne_id) as *mut NetworkEntity;
                if !ptr.is_null() {
                    unsafe {
                        (*ptr).net_id = rpc.net_id;
                        (*ptr).set_prefab(&rpc.prefab);
                    }
                }
            }
            self.map.insert(rpc.net_id, entity);
        } else {
            self.static_entities.push(entity);
        }

        for (path, value) in &rpc.hot {
            apply_field(world, entity, *path, value);
        }
        // Also apply transform-related cold overrides that aren't PrefabDefault
        for (path, value) in &rpc.cold {
            if matches!(
                path,
                FieldPathId::TransformPosition
                    | FieldPathId::TransformRotation
                    | FieldPathId::TransformScale
            ) {
                apply_field(world, entity, *path, value);
            }
        }

        world.simulate(0.0);
        log::info!(
            "[prefab_rep] client spawn prefab='{}' net_id={} ent={:#x}",
            rpc.prefab,
            rpc.net_id,
            entity
        );
        Ok(())
    }

    fn apply_delta(&mut self, world: &World, rpc: NetDeltaRpc) {
        let Some(&entity) = self.map.get(&rpc.net_id) else {
            log::warn!("[prefab_rep] delta unknown net_id={}", rpc.net_id);
            return;
        };
        for (path, value) in &rpc.fields {
            apply_field(world, entity, *path, value);
        }
    }

    fn apply_destroy(&mut self, world: &World, rpc: NetDestroyRpc) {
        if let Some(entity) = self.map.remove(&rpc.net_id) {
            world.destroy(entity);
            world.simulate(0.0);
            log::info!("[prefab_rep] client destroy net_id={}", rpc.net_id);
        }
    }
}
