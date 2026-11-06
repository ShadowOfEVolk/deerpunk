//! Client entity streaming: `stream.in` / `stream.out` / batched `stream.update`.

use std::collections::HashMap;
use std::path::Path;

use deerpunk_engine_common::components::NetworkEntity;
use deerpunk_engine_common::network::field_apply::apply_field_ex;
use deerpunk_engine_common::network::field_path::{FieldValue, NetFieldId};
use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::{
    NetDeltaRpc, NetDestroyRpc, PrefabDefRpc, PrefabNetId, PrefabNetRegistry, SpawnPrefabRpc,
    StreamInRpc, StreamOutRpc, StreamUpdateBatchRpc,
};
use deerpunk_engine_common::scene::PrefabManager;
use deerpunk_engine_common::scene::spawn_prefab_filtered_with_component_overrides_cached;
use deerpunk_engine_common::resource::{MeshRegistry, ResourceManager};
use deerpunk_engine_common::world::{EntityId, World};

use crate::client_engine::ClientEngine;
use crate::draw_material_resolve::resolve_draw_material_uri;

/// Server-authoritative sim; client replica uses [`LayoutMetaRegistry::client_receives_prefab_component`].
fn client_prefab_component_allowed(world: &World, name: &str) -> bool {
    world
        .layout_registry()
        .client_receives_prefab_component(name)
}

#[derive(Default)]
pub struct ClientPrefabReplication {
    network_prefabs: PrefabManager,
    prefab_ids: PrefabNetRegistry,
    /// net_id → (client entity, stream generation)
    map: HashMap<u16, (EntityId, u8)>,
    static_entities: Vec<EntityId>,
    view_net_id: Option<u16>,
}

impl ClientPrefabReplication {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_stream_state(&mut self) {
        self.map.clear();
        self.static_entities.clear();
        self.view_net_id = None;
        self.network_prefabs.clear_cache();
        self.prefab_ids.clear();
    }

    pub fn replicated_entities(&self) -> Vec<EntityId> {
        let mut entities: Vec<EntityId> = self.map.values().map(|(e, _)| *e).collect();
        for &e in &self.static_entities {
            if !entities.contains(&e) {
                entities.push(e);
            }
        }
        entities
    }

    /// Re-resolve `Mesh3D` / `DrawMaterial` URIs (e.g. after BSP session is ready).
    pub fn refresh_mesh_uri_overrides(
        &self,
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
    ) {
        for entity in self.replicated_entities() {
            let _ = deerpunk_engine_common::scene::resolve_mesh3d_uri(
                &client.world,
                meshes,
                gamedir,
                resources,
                entity,
            );
            let _ = resolve_draw_material_uri(client, resources, entity, false);
            client.mesh_adopt_entity(meshes, entity);
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
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        packet: &[u8],
    ) -> bool {
        let world = &client.world;
        let Some((&cmd, payload)) = packet.split_first() else {
            return false;
        };
        if cmd == SVC::StreamIn as u8 {
            match StreamInRpc::decode(payload) {
                Some(rpc) => {
                    if let Err(e) = self.apply_stream_in(client, meshes, gamedir, resources, rpc) {
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
            match SpawnPrefabRpc::decode(payload) {
                Some(rpc) => {
                    log::warn!("[stream] legacy SpawnPrefab ignored (use StreamIn)");
                    let _ = rpc;
                    true
                }
                None => {
                    log::warn!("[stream] bad SpawnPrefab");
                    true
                }
            }
        } else if cmd == SVC::StreamOut as u8 {
            match StreamOutRpc::decode(payload) {
                Some(rpc) => {
                    self.apply_stream_out(client, meshes, rpc.net_id);
                    true
                }
                None => {
                    log::warn!("[stream] bad stream.out");
                    true
                }
            }
        } else if cmd == SVC::StreamUpdate as u8 {
            match StreamUpdateBatchRpc::decode(payload) {
                Some(batch) => {
                    self.apply_stream_update_batch(client, meshes, gamedir, resources, batch);
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
                    self.apply_entity_update(client, meshes, gamedir, resources, rpc.net_id, &rpc.fields);
                    true
                }
                None => {
                    log::warn!("[stream] bad NetDelta");
                    true
                }
            }
        } else if cmd == SVC::NetDestroy as u8 {
            match NetDestroyRpc::decode(payload) {
                Some(rpc) => {
                    self.apply_stream_out(client, meshes, rpc.net_id);
                    true
                }
                None => {
                    log::warn!("[stream] bad NetDestroy");
                    true
                }
            }
        } else if cmd == SVC::PrefabDef as u8 {
            match PrefabDefRpc::decode(payload) {
                Some(rpc) => {
                    if let Err(e) = self.prefab_ids.register(rpc.prefab_id, &rpc.name) {
                        log::warn!("[stream] PrefabDef id: {e}");
                    }
                    self.network_prefabs.insert_cached(&rpc.name, rpc.def);
                    log::info!(
                        "[stream] PrefabDef id={} '{}'",
                        rpc.prefab_id.0,
                        rpc.name
                    );
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
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        rpc: StreamInRpc,
    ) -> Result<(), String> {
        let prefab = self
            .prefab_ids
            .name_of(rpc.prefab_id)
            .ok_or_else(|| format!("unknown prefab_id {}", rpc.prefab_id.0))?
            .to_string();
        let trace_drop = prefab == "item_drop";
        if trace_drop {
            log::info!(
                "[stream/item_drop] 1 begin net_id={} gen={} prefab_id={} cold={} hot={}",
                rpc.net_id,
                rpc.generation,
                rpc.prefab_id.0,
                rpc.cold.len(),
                rpc.hot.len()
            );
        }
        if rpc.net_id != 0 {
            if let Some((old, gen)) = self.map.get(&rpc.net_id) {
                if gen == &rpc.generation {
                    return Ok(());
                }
                Self::destroy_client_entity(client, meshes, *old);
            }
        }

        let mut allow_keys: Vec<String> = vec!["TransformComponent".into()];
        if rpc.net_id != 0 {
            allow_keys.push("NetworkEntity".into());
        }
        if let Some(def) = self.network_prefabs.get_cached(&prefab) {
            for name in def.components.keys() {
                if !client_prefab_component_allowed(&client.world, name) {
                    continue;
                }
                if !allow_keys.iter().any(|x| x == name) {
                    allow_keys.push(name.clone());
                }
            }
        }
        allow_keys.retain(|k| client_prefab_component_allowed(&client.world, k));
        let allow: Vec<&str> = allow_keys.iter().map(|s| s.as_str()).collect();
        let empty_overrides = HashMap::new();

        if trace_drop {
            log::info!("[stream/item_drop] 2 spawn_prefab allow={allow_keys:?}");
        }
        let entity = spawn_prefab_filtered_with_component_overrides_cached(
            &client.world,
            meshes,
            gamedir,
            resources,
            &prefab,
            Some(&allow),
            &empty_overrides,
            Some(&self.network_prefabs),
        )
        .map_err(|e| e.message)?;

        if trace_drop {
            log::info!("[stream/item_drop] 3 spawned ent={entity:#x}");
        }

        if rpc.net_id != 0 {
            let ne_id = client.world.component_init::<NetworkEntity>();
            if client.world.has(entity, ne_id) {
                let ptr = client.world.get_raw(entity, ne_id) as *mut NetworkEntity;
                if !ptr.is_null() {
                    unsafe {
                        (*ptr).net_id = rpc.net_id;
                        (*ptr).set_prefab(&prefab);
                    }
                }
            }
            self.map.insert(rpc.net_id, (entity, rpc.generation));
        } else {
            self.static_entities.push(entity);
        }

        if trace_drop {
            log::info!("[stream/item_drop] 4 cold_fields");
        }
        self.apply_net_fields(client, meshes, gamedir, resources, entity, &rpc.cold);

        if trace_drop {
            log::info!("[stream/item_drop] 5 hot_fields");
        }
        self.apply_net_fields(client, meshes, gamedir, resources, entity, &rpc.hot);

        if trace_drop {
            log::info!("[stream/item_drop] 6 resolve_draw_material");
        }
        let _ = resolve_draw_material_uri(client, resources, entity, false);
        if trace_drop {
            log::info!("[stream/item_drop] 7 mesh_adopt");
        }
        client.mesh_adopt_entity(meshes, entity);
        if trace_drop {
            log::info!("[stream/item_drop] 8 done");
        }
        log::info!(
            "[stream] in prefab='{}' id={} net_id={} gen={} ent={:#x} cold={} hot={}",
            prefab,
            rpc.prefab_id.0,
            rpc.net_id,
            rpc.generation,
            entity,
            rpc.cold.len(),
            rpc.hot.len()
        );
        Ok(())
    }

    fn destroy_client_entity(
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        entity: EntityId,
    ) {
        client.mesh_release_entity(meshes, entity);
        client.drop_heap_mesh_components(meshes, entity);
        client.world.destroy(entity);
    }

    fn apply_stream_out(
        &mut self,
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        net_id: u16,
    ) {
        if let Some((entity, _)) = self.map.remove(&net_id) {
            Self::destroy_client_entity(client, meshes, entity);
            log::info!("[stream] out net_id={}", net_id);
        }
    }

    fn apply_stream_update_batch(
        &mut self,
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        batch: StreamUpdateBatchRpc,
    ) {
        for u in batch.updates {
            self.apply_entity_update(client, meshes, gamedir, resources, u.net_id, &u.fields);
        }
    }

    fn apply_entity_update(
        &mut self,
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        net_id: u16,
        fields: &[(NetFieldId, FieldValue)],
    ) {
        let Some(&(entity, _)) = self.map.get(&net_id) else {
            log::warn!("[stream] update unknown net_id={}", net_id);
            return;
        };
        self.apply_hot_fields(client, meshes, gamedir, resources, entity, fields);
    }

    fn apply_net_fields(
        &self,
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        entity: EntityId,
        fields: &[(NetFieldId, FieldValue)],
    ) {
        use glam::Vec3;
        let world = &client.world;
        for (path, value) in fields {
            if matches!(value, FieldValue::PrefabDefault) {
                continue;
            }
            if client.local_sim.should_skip_transform_pos(entity)
                && *path == NetFieldId::TRANSFORM_POSITION
            {
                if let FieldValue::Vec3(v) = value {
                    client.local_sim.set_server_feet(Vec3::from_array(*v));
                }
                continue;
            }
            apply_field_ex(world, Some(meshes), Some(gamedir), Some(resources), entity, *path, value);
        }
    }

    fn apply_hot_fields(
        &self,
        client: &mut ClientEngine,
        meshes: &mut MeshRegistry,
        gamedir: &Path,
        resources: &ResourceManager,
        entity: EntityId,
        fields: &[(NetFieldId, FieldValue)],
    ) {
        self.apply_net_fields(client, meshes, gamedir, resources, entity, fields);
    }
}
