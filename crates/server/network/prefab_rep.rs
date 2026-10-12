//! Server-side prefab replication: baseline spawn + hot deltas.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use deerpunk_engine_common::components::{NetworkEntity, Worldspawn};
use deerpunk_engine_common::network::field_apply::read_field;
use deerpunk_engine_common::network::field_path::{FieldPathId, FieldValue};
use deerpunk_engine_common::network::{NetDeltaRpc, NetDestroyRpc, PrefabDefRpc, SpawnPrefabRpc};
use deerpunk_engine_common::scene::PrefabManager;
use deerpunk_engine_common::world::{EntityId, World};

#[derive(Default)]
pub struct ServerPrefabReplication {
    next_net_id: u16,
    /// server entity → net_id
    entity_to_net: HashMap<EntityId, u16>,
    /// net_id → server entity
    net_to_entity: HashMap<u16, EntityId>,
    /// last encoded hot snapshot per net_id
    last_hot: HashMap<u16, Vec<(FieldPathId, FieldValue)>>,
    /// worldspawn entities already baseline-spawned (no net_id)
    worldspawn_sent: Vec<EntityId>,
}

impl ServerPrefabReplication {
    pub fn new() -> Self {
        Self {
            next_net_id: 1,
            ..Default::default()
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn net_id_of(&self, entity: EntityId) -> Option<u16> {
        self.entity_to_net.get(&entity).copied()
    }

    /// Full baseline after map load / reconnect.
    pub fn baseline(&mut self, world: &World) -> Vec<Vec<u8>> {
        self.clear();
        let mut packets = Vec::new();
        packets.extend(self.emit_worldspawns(world));
        packets.extend(self.emit_network_spawns(world, true));
        packets
    }

    /// Prefab bodies for all classnames referenced in the current world (connect init).
    pub fn prefab_definition_packets(&self, gamedir: &Path, world: &World) -> Vec<Vec<u8>> {
        let names = self.collect_prefab_names(world);
        let mut pm = PrefabManager::new();
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            match pm.resolve(gamedir, &name) {
                Ok(def) => out.push(PrefabDefRpc { name, def }.to_packet()),
                Err(e) => log::warn!("[prefab_rep] skip def '{name}': {e}"),
            }
        }
        out
    }

    fn collect_prefab_names(&self, world: &World) -> Vec<String> {
        let mut set = HashSet::new();
        let ne_id = world.component_init::<NetworkEntity>();
        for e in world.inspect_entities() {
            if !world.has(e.id, ne_id) {
                continue;
            }
            let ptr = world.get_raw(e.id, ne_id) as *const NetworkEntity;
            if ptr.is_null() {
                continue;
            }
            let prefab = unsafe { &*ptr }.prefab_str();
            if !prefab.is_empty() {
                set.insert(prefab.to_string());
            }
        }
        let mut names: Vec<String> = set.into_iter().collect();
        names.sort();
        names
    }

    /// Join init: replicated `NetworkEntity` spawns only (map geometry via `UserMessage Map`).
    pub fn snapshot(&self, world: &World) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        let ne_id = world.component_init::<NetworkEntity>();
        for e in world.inspect_entities() {
            if !world.has(e.id, ne_id) {
                continue;
            }
            let Some(&net_id) = self.entity_to_net.get(&e.id) else {
                continue;
            };
            let ptr = world.get_raw(e.id, ne_id) as *const NetworkEntity;
            if ptr.is_null() {
                continue;
            }
            let net = unsafe { &*ptr };
            let prefab = net.prefab_str();
            if prefab.is_empty() {
                continue;
            }
            let cold: Vec<(FieldPathId, FieldValue)> = net
                .cold_paths()
                .map(|p| (p, FieldValue::PrefabDefault))
                .collect();
            let hot: Vec<(FieldPathId, FieldValue)> = net
                .hot_paths()
                .filter_map(|p| read_field(world, e.id, p).map(|v| (p, v)))
                .collect();
            packets.push(
                SpawnPrefabRpc {
                    net_id,
                    prefab: prefab.to_string(),
                    cold,
                    hot,
                }
                .to_packet(),
            );
        }
        packets
    }

    /// Per-frame: new NetworkEntity spawns, hot deltas, destroys.
    pub fn tick(&mut self, world: &World) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        packets.extend(self.emit_worldspawns(world));
        packets.extend(self.emit_network_spawns(world, false));
        packets.extend(self.emit_hot_deltas(world));
        packets.extend(self.emit_destroys(world));
        packets
    }

    fn alloc_net_id(&mut self) -> u16 {
        let id = self.next_net_id;
        self.next_net_id = self.next_net_id.wrapping_add(1);
        if self.next_net_id == 0 {
            self.next_net_id = 1;
        }
        id
    }

    fn emit_worldspawns(&mut self, world: &World) -> Vec<Vec<u8>> {
        let ws_id = world.component_init::<Worldspawn>();
        let mut out = Vec::new();
        for e in world.inspect_entities() {
            if !world.has(e.id, ws_id) {
                continue;
            }
            if self.worldspawn_sent.contains(&e.id) {
                continue;
            }
            let map = match read_field(world, e.id, FieldPathId::WorldspawnMap) {
                Some(FieldValue::Str(s)) => s,
                _ => continue,
            };
            let mut cold = vec![
                (FieldPathId::Worldspawn, FieldValue::PrefabDefault),
                (FieldPathId::Material, FieldValue::PrefabDefault),
                (
                    FieldPathId::TransformPosition,
                    read_field(world, e.id, FieldPathId::TransformPosition)
                        .unwrap_or(FieldValue::Vec3([0.0, 0.0, 0.0])),
                ),
                (FieldPathId::WorldspawnMap, FieldValue::Str(map)),
            ];
            // Transform scale if present
            if let Some(s) = read_field(world, e.id, FieldPathId::TransformScale) {
                cold.push((FieldPathId::TransformScale, s));
            }
            let rpc = SpawnPrefabRpc {
                net_id: 0,
                prefab: "worldspawn".into(),
                cold,
                hot: Vec::new(),
            };
            out.push(rpc.to_packet());
            self.worldspawn_sent.push(e.id);
        }
        out
    }

    fn emit_network_spawns(&mut self, world: &World, force_all: bool) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let mut out = Vec::new();
        for e in world.inspect_entities() {
            if !world.has(e.id, ne_id) {
                continue;
            }
            if !force_all && self.entity_to_net.contains_key(&e.id) {
                continue;
            }
            if force_all && self.entity_to_net.contains_key(&e.id) {
                continue;
            }

            let ptr = world.get_raw(e.id, ne_id) as *mut NetworkEntity;
            if ptr.is_null() {
                continue;
            }
            let net = unsafe { &mut *ptr };
            let prefab = {
                let s = net.prefab_str();
                if s.is_empty() {
                    log::warn!(
                        "[prefab_rep] NetworkEntity {:#x} has empty prefab name — skip",
                        e.id
                    );
                    continue;
                }
                s.to_string()
            };

            let net_id = if net.net_id != 0 {
                net.net_id
            } else {
                self.alloc_net_id()
            };
            net.net_id = net_id;

            let cold: Vec<(FieldPathId, FieldValue)> = net
                .cold_paths()
                .map(|p| (p, FieldValue::PrefabDefault))
                .collect();

            let hot: Vec<(FieldPathId, FieldValue)> = net
                .hot_paths()
                .filter_map(|p| read_field(world, e.id, p).map(|v| (p, v)))
                .collect();

            self.entity_to_net.insert(e.id, net_id);
            self.net_to_entity.insert(net_id, e.id);
            self.last_hot.insert(net_id, hot.clone());

            let rpc = SpawnPrefabRpc {
                net_id,
                prefab,
                cold,
                hot,
            };
            out.push(rpc.to_packet());
        }
        out
    }

    fn emit_hot_deltas(&mut self, world: &World) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let mut out = Vec::new();
        let tracked: Vec<(EntityId, u16)> = self.entity_to_net.iter().map(|(&e, &n)| (e, n)).collect();
        for (entity, net_id) in tracked {
            if !world.has(entity, ne_id) {
                continue;
            }
            let ptr = world.get_raw(entity, ne_id) as *const NetworkEntity;
            if ptr.is_null() {
                continue;
            }
            let net = unsafe { &*ptr };
            let mut changed = Vec::new();
            let prev = self.last_hot.get(&net_id).cloned().unwrap_or_default();
            for path in net.hot_paths() {
                let Some(val) = read_field(world, entity, path) else {
                    continue;
                };
                let old = prev.iter().find(|(p, _)| *p == path).map(|(_, v)| v);
                if old.map(|o| field_eq(o, &val)).unwrap_or(false) {
                    continue;
                }
                changed.push((path, val));
            }
            if changed.is_empty() {
                continue;
            }
            // Update snapshot
            let snap = self.last_hot.entry(net_id).or_default();
            for (path, val) in &changed {
                if let Some(slot) = snap.iter_mut().find(|(p, _)| *p == *path) {
                    slot.1 = val.clone();
                } else {
                    snap.push((*path, val.clone()));
                }
            }
            out.push(
                NetDeltaRpc {
                    net_id,
                    fields: changed,
                }
                .to_packet(),
            );
        }
        out
    }

    fn emit_destroys(&mut self, world: &World) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let mut dead = Vec::new();
        for (&entity, &net_id) in &self.entity_to_net {
            if !world.has(entity, ne_id) {
                dead.push((entity, net_id));
            }
        }
        let mut out = Vec::new();
        for (entity, net_id) in dead {
            self.entity_to_net.remove(&entity);
            self.net_to_entity.remove(&net_id);
            self.last_hot.remove(&net_id);
            out.push(NetDestroyRpc { net_id }.to_packet());
        }
        out
    }
}

fn field_eq(a: &FieldValue, b: &FieldValue) -> bool {
    match (a, b) {
        (FieldValue::Vec3(x), FieldValue::Vec3(y)) => x == y,
        (FieldValue::Quat(x), FieldValue::Quat(y)) => x == y,
        (FieldValue::Str(x), FieldValue::Str(y)) => x == y,
        (FieldValue::PrefabDefault, FieldValue::PrefabDefault) => true,
        _ => false,
    }
}
