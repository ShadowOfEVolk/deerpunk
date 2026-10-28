//! Server-side prefab replication: baseline spawn + hot deltas.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use deerpunk_engine_common::components::NetworkEntity;
use deerpunk_engine_common::network::field_apply::read_field;
use deerpunk_engine_common::network::build_stream_in_components;
use deerpunk_engine_common::network::field_path::{FieldValue, NetFieldId};
use deerpunk_engine_common::network::{
    PrefabDefRpc, StreamEntityUpdate, StreamInRpc, StreamOutRpc, StreamUpdateBatchRpc,
};
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
    last_hot: HashMap<u16, Vec<(NetFieldId, FieldValue)>>,
    /// `net_id` → stream generation (bumps on each `stream.in`).
    generation: HashMap<u16, u8>,
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
        self.emit_network_spawns(world, true)
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
        world.for_each_with_component(ne_id, |entity| {
            let ptr = world.get_raw(entity, ne_id) as *const NetworkEntity;
            if ptr.is_null() {
                return;
            }
            let prefab = unsafe { &*ptr }.prefab_str();
            if !prefab.is_empty() {
                set.insert(prefab.to_string());
            }
        });
        let mut names: Vec<String> = set.into_iter().collect();
        names.sort();
        names
    }

    /// Join init: `stream.in` for every [`NetworkEntity`] (worldspawn, player, func_*, …).
    pub fn snapshot(&mut self, world: &World) -> Vec<Vec<u8>> {
        self.emit_network_spawns(world, true)
    }

    /// Per-frame: new NetworkEntity spawns, hot deltas, destroys.
    pub fn tick(&mut self, world: &World) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
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

    fn emit_network_spawns(&mut self, world: &World, resend_all: bool) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let mut out = Vec::new();
        world.for_each_with_component(ne_id, |entity| {
            if !resend_all && self.entity_to_net.contains_key(&entity) {
                return;
            }

            let ptr = world.get_raw(entity, ne_id) as *mut NetworkEntity;
            if ptr.is_null() {
                return;
            }
            let net = unsafe { &mut *ptr };
            let prefab = {
                let s = net.prefab_str();
                if s.is_empty() {
                    log::warn!(
                        "[prefab_rep] NetworkEntity {:#x} has empty prefab name — skip",
                        entity
                    );
                    return;
                }
                s.to_string()
            };

            let net_id = if let Some(&id) = self.entity_to_net.get(&entity) {
                id
            } else if net.net_id != 0 {
                net.net_id
            } else {
                self.alloc_net_id()
            };
            net.net_id = net_id;

            let components = build_stream_in_components(world, entity, net);
            let hot: Vec<_> = net
                .hot_paths()
                .filter_map(|p| read_field(world, entity, p).map(|v| (p, v)))
                .collect();

            self.entity_to_net.insert(entity, net_id);
            self.net_to_entity.insert(net_id, entity);
            self.last_hot.insert(net_id, hot.clone());
            let generation = *self
                .generation
                .entry(net_id)
                .and_modify(|g| *g = g.wrapping_add(1))
                .or_insert(0);

            let rpc = StreamInRpc {
                net_id,
                generation,
                prefab,
                components,
                hot,
            };
            out.push(rpc.to_packet());
        });
        out
    }

    fn emit_hot_deltas(&mut self, world: &World) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let mut batch = Vec::new();
        let tracked: Vec<(EntityId, u16)> =
            self.entity_to_net.iter().map(|(&e, &n)| (e, n)).collect();
        for (entity, net_id) in tracked {
            if !world.has(entity, ne_id) {
                continue;
            }
            let ptr = world.get_raw(entity, ne_id) as *const NetworkEntity;
            if ptr.is_null() {
                continue;
            }
            let net = unsafe { &*ptr };
            if net.is_dormant() {
                continue;
            }
            let mut changed = Vec::new();
            let prev = self.last_hot.get(&net_id);
            for path in net.hot_paths() {
                let Some(val) = read_field(world, entity, path) else {
                    continue;
                };
                let old = prev
                    .and_then(|snap| snap.iter().find(|(p, _)| *p == path))
                    .map(|(_, v)| v);
                if old.map(|o| field_eq(o, &val)).unwrap_or(false) {
                    continue;
                }
                changed.push((path, val));
            }
            if changed.is_empty() {
                continue;
            }
            let snap = self.last_hot.entry(net_id).or_default();
            for (path, val) in &changed {
                if let Some(slot) = snap.iter_mut().find(|(p, _)| *p == *path) {
                    slot.1 = val.clone();
                } else {
                    snap.push((*path, val.clone()));
                }
            }
            batch.push(StreamEntityUpdate {
                net_id,
                fields: changed,
            });
        }
        if batch.is_empty() {
            Vec::new()
        } else {
            vec![StreamUpdateBatchRpc { updates: batch }.to_packet()]
        }
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
            self.generation.remove(&net_id);
            out.push(StreamOutRpc { net_id }.to_packet());
        }
        out
    }
}

/// Ignore sub-millimeter jitter on replicated transforms (20 Hz physics noise).
const REPL_VEC3_EPS: f32 = 0.002;

fn vec3_near(a: &[f32; 3], b: &[f32; 3]) -> bool {
    (a[0] - b[0]).abs() <= REPL_VEC3_EPS
        && (a[1] - b[1]).abs() <= REPL_VEC3_EPS
        && (a[2] - b[2]).abs() <= REPL_VEC3_EPS
}

fn field_eq(a: &FieldValue, b: &FieldValue) -> bool {
    match (a, b) {
        (FieldValue::Vec3(x), FieldValue::Vec3(y)) => vec3_near(x, y),
        (FieldValue::Quat(x), FieldValue::Quat(y)) => x == y,
        (FieldValue::Str(x), FieldValue::Str(y)) => x == y,
        (FieldValue::PrefabDefault, FieldValue::PrefabDefault) => true,
        (FieldValue::U32(x), FieldValue::U32(y)) => x == y,
        _ => false,
    }
}
