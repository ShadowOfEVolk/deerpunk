//! Client entity streaming: `stream.in` / `stream.out` / batched `stream.update`.

use std::collections::HashMap;
use std::path::Path;

use deerpunk_engine_common::components::NetworkEntity;
use deerpunk_engine_common::network::field_apply::apply_field_ex;
use deerpunk_engine_common::network::field_path::{FieldPathId, FieldValue};
use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::{
    NetDeltaRpc, NetDestroyRpc, PrefabDefRpc, SpawnPrefabRpc, StreamInRpc, StreamOutRpc,
    StreamUpdateBatchRpc,
};
use deerpunk_engine_common::scene::PrefabManager;
use deerpunk_engine_common::scene::spawn_prefab_filtered_with_overrides_cached;
use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::{EntityId, World};

#[derive(Default)]
pub struct ClientPrefabReplication {
    network_prefabs: PrefabManager,
    /// net_id → (client entity, stream generation)
    map: HashMap<u16, (EntityId, u8)>,
    static_entities: Vec<EntityId>,
    view_net_id: Option<u16>,
    /// Last mesh URI overrides from `stream.in` (re-apply after map load).
    mesh_uri_by_net: HashMap<u16, String>,
}

impl ClientPrefabReplication {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self, world: &World) {
        for (_, (ent, _)) in self.map.drain() {
            world.destroy(ent);
        }
        for &id in &self.static_entities {
            world.destroy(id);
        }
        self.static_entities.clear();
        self.view_net_id = None;
        self.mesh_uri_by_net.clear();
        self.network_prefabs.clear_cache();
        world.simulate(0.0);
    }

    /// Re-apply stored `Mesh3D.mesh` URIs (e.g. after BSP submodel cache is ready).
    pub fn refresh_mesh_uri_overrides(
        &self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
    ) {
        for (&net_id, uri) in &self.mesh_uri_by_net {
            let Some(&(entity, _)) = self.map.get(&net_id) else {
                continue;
            };
            let path = FieldPathId::Mesh3DMesh;
            let value = FieldValue::Str(uri.clone());
            apply_field_ex(world, Some(gamedir), Some(resources), entity, path, &value);
        }
    }

    pub fn client_of_net(&self, net_id: u16) -> Option<EntityId> {
        self.map.get(&net_id).map(|(e, _)| *e)
    }

    pub fn view_net_id(&self) -> Option<u16> {
        self.view_net_id
    }

    pub fn view_entity(&self) -> Option<EntityId> {
        self.view_net_id.and_then(|id| self.client_of_net(id))
    }

    pub fn set_view_net_id(&mut self, net_id: u16) {
        self.view_net_id = if net_id == 0 { None } else { Some(net_id) };
        log::info!("[stream] SetView net_id={net_id}");
    }

    pub fn track_static_entity(&mut self, entity: EntityId) {
        self.static_entities.push(entity);
    }

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
        if cmd == SVC::StreamIn as u8 {
            match StreamInRpc::decode(payload) {
                Some(rpc) => {
                    if let Err(e) = self.apply_stream_in(world, gamedir, resources, rpc) {
                        log::error!("[stream] in: {e}");
                    }
                    true
                }
                None => {
                    log::warn!("[stream] bad stream.in");
                    true
                }
            }
        } else if cmd == SVC::SpawnPrefab as u8 {
            // Legacy join packets (`generation` implied 0).
            match SpawnPrefabRpc::decode(payload) {
                Some(spawn) => {
                    let rpc = StreamInRpc {
                        net_id: spawn.net_id,
                        generation: 0,
                        prefab: spawn.prefab,
                        cold: spawn.cold,
                        hot: spawn.hot,
                    };
                    if let Err(e) = self.apply_stream_in(world, gamedir, resources, rpc) {
                        log::error!("[stream] legacy spawn: {e}");
                    }
                    true
                }
                None => {
                    log::warn!("[stream] bad legacy SpawnPrefab");
                    true
                }
            }
        } else if cmd == SVC::StreamOut as u8 || cmd == SVC::NetDestroy as u8 {
            let net_id = if cmd == SVC::StreamOut as u8 {
                StreamOutRpc::decode(payload).map(|r| r.net_id)
            } else {
                NetDestroyRpc::decode(payload).map(|r| r.net_id)
            };
            if let Some(net_id) = net_id {
                self.apply_stream_out(world, net_id);
            }
            true
        } else if cmd == SVC::StreamUpdate as u8 {
            match StreamUpdateBatchRpc::decode(payload) {
                Some(batch) => {
                    self.apply_stream_update_batch(world, gamedir, resources, batch);
                    true
                }
                None => {
                    log::warn!("[stream] bad stream.update");
                    true
                }
            }
        } else if cmd == SVC::NetDelta as u8 {
            match NetDeltaRpc::decode(payload) {
                Some(rpc) => {
                    self.apply_entity_update(world, gamedir, resources, rpc.net_id, &rpc.fields);
                    true
                }
                None => {
                    log::warn!("[stream] bad legacy NetDelta");
                    true
                }
            }
        } else if cmd == SVC::PrefabDef as u8 {
            match PrefabDefRpc::decode(payload) {
                Some(rpc) => {
                    self.network_prefabs.insert_cached(&rpc.name, rpc.def);
                    log::info!("[stream] PrefabDef '{}'", rpc.name);
                    true
                }
                None => {
                    log::warn!("[stream] bad PrefabDef");
                    true
                }
            }
        } else if cmd == SVC::UserMessage as u8 {
            false
        } else {
            false
        }
    }

    fn apply_stream_in(
        &mut self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        rpc: StreamInRpc,
    ) -> Result<(), String> {
        if rpc.net_id != 0 {
            if let Some((old, gen)) = self.map.get(&rpc.net_id) {
                if gen == &rpc.generation {
                    return Ok(());
                }
                world.destroy(*old);
            }
        }

        let mut allow_keys: Vec<&str> = vec!["TransformComponent"];
        for (path, _) in rpc.cold.iter().chain(rpc.hot.iter()) {
            if let Some(k) = path.component_key() {
                if !allow_keys.contains(&k) {
                    allow_keys.push(k);
                }
            }
        }
        if rpc.net_id != 0 {
            allow_keys.push("NetworkEntity");
        }
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
            self.map.insert(rpc.net_id, (entity, rpc.generation));
        } else {
            self.static_entities.push(entity);
        }

        self.apply_overrides(world, gamedir, resources, entity, &rpc.hot);
        self.apply_overrides(world, gamedir, resources, entity, &rpc.cold);

        if rpc.net_id != 0 {
            if let Some(uri) = rpc
                .cold
                .iter()
                .chain(rpc.hot.iter())
                .find(|(p, _)| *p == FieldPathId::Mesh3DMesh)
                .and_then(|(_, v)| match v {
                    FieldValue::Str(s) => Some(s.clone()),
                    _ => None,
                })
            {
                self.mesh_uri_by_net.insert(rpc.net_id, uri);
            }
        }

        world.simulate(0.0);
        log::info!(
            "[stream] in prefab='{}' net_id={} gen={} ent={:#x}",
            rpc.prefab,
            rpc.net_id,
            rpc.generation,
            entity
        );
        Ok(())
    }

    fn apply_stream_out(&mut self, world: &World, net_id: u16) {
        if let Some((entity, _)) = self.map.remove(&net_id) {
            world.destroy(entity);
            world.simulate(0.0);
            log::info!("[stream] out net_id={}", net_id);
        }
    }

    fn apply_stream_update_batch(
        &mut self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        batch: StreamUpdateBatchRpc,
    ) {
        for u in batch.updates {
            self.apply_entity_update(world, gamedir, resources, u.net_id, &u.fields);
        }
    }

    fn apply_entity_update(
        &mut self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        net_id: u16,
        fields: &[(FieldPathId, FieldValue)],
    ) {
        let Some(&(entity, _)) = self.map.get(&net_id) else {
            log::warn!("[stream] update unknown net_id={}", net_id);
            return;
        };
        self.apply_overrides(world, gamedir, resources, entity, fields);
    }

    fn apply_overrides(
        &self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        entity: EntityId,
        fields: &[(FieldPathId, FieldValue)],
    ) {
        for (path, value) in fields {
            if matches!(value, FieldValue::PrefabDefault) {
                continue;
            }
            apply_field_ex(world, Some(gamedir), Some(resources), entity, *path, value);
        }
    }
}
