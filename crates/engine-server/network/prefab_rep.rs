//! Server-side prefab replication: baseline spawn + hot deltas.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use deerpunk_engine_common::components::NetworkEntity;
use deerpunk_engine_common::network::field_apply::{read_field, stream_in_field_snapshot};
use deerpunk_engine_common::network::field_path::{FieldValue, NetFieldId};
use deerpunk_engine_common::network::{mark_replication_dirty, take_replication_dirty};
use deerpunk_engine_common::network::{
    PrefabDefRpc, PrefabNetId, PrefabNetRegistry, StreamEntityUpdate, StreamInRpc, StreamOutRpc,
    StreamUpdateBatchRpc,
};
use deerpunk_engine_common::scene::PrefabManager;
use deerpunk_engine_common::world::{EntityId, WatchEvent, World};

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
    prefab_ids: PrefabNetRegistry,
    /// Extra classnames registered before connect (dynamic spawns).
    catalog_prefabs: HashSet<String>,
    /// Flecs `OnSet` observer registered (hot deltas are dirty-set driven).
    change_tracking: bool,
    observer_checked: bool,
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

    /// Register a classname so its [`PrefabDefRpc`] is sent at connect even if no entity exists yet.
    pub fn catalog_prefab(&mut self, classname: &str) {
        let name = classname.trim();
        if !name.is_empty() {
            self.catalog_prefabs.insert(name.to_string());
        }
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
    pub fn prefab_definition_packets(&mut self, gamedir: &Path, world: &World) -> Vec<Vec<u8>> {
        self.prefab_ids.clear();
        let names = self.collect_prefab_catalog(gamedir, world);
        let assigned = PrefabNetRegistry::assign_ids_sorted(&names);
        let mut pm = PrefabManager::new();
        let mut out = Vec::with_capacity(assigned.len());
        for (prefab_id, name) in assigned {
            if let Err(e) = self.prefab_ids.register(prefab_id, &name) {
                log::warn!("[prefab_rep] id map: {e}");
                continue;
            }
            match pm.resolve(gamedir, &name) {
                Ok(def) => out.push(
                    PrefabDefRpc {
                        prefab_id,
                        name,
                        def,
                    }
                    .to_packet(),
                ),
                Err(e) => log::warn!("[prefab_rep] skip def '{name}': {e}"),
            }
        }
        out
    }

    fn collect_prefab_catalog(&self, gamedir: &Path, world: &World) -> Vec<String> {
        let mut set: HashSet<String> = self.collect_prefab_names(world).into_iter().collect();
        set.extend(self.catalog_prefabs.iter().cloned());
        for name in PrefabManager::list_disk_classnames(gamedir) {
            set.insert(name);
        }
        let mut names: Vec<String> = set.into_iter().collect();
        names.sort();
        names
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

    /// Per-frame: new NetworkEntity spawns, destroys (before hot), hot deltas.
    pub fn tick(&mut self, world: &World) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        packets.extend(self.emit_network_spawns(world, false));
        packets.extend(self.emit_destroys(world));
        packets.extend(self.emit_hot_deltas(world));
        packets
    }

    fn drop_binding(&mut self, entity: EntityId, net_id: u16) {
        self.entity_to_net.remove(&entity);
        self.net_to_entity.remove(&net_id);
        self.last_hot.remove(&net_id);
        self.generation.remove(&net_id);
    }

    /// True when `entity_to_net` still refers to this live [`NetworkEntity`] (guards id reuse).
    fn binding_valid(&self, world: &World, entity: EntityId, net_id: u16, ne_id: u64) -> bool {
        if entity == 0 || !world.is_alive(entity) || !world.has(entity, ne_id) {
            return false;
        }
        if self.net_to_entity.get(&net_id) != Some(&entity) {
            return false;
        }
        let ptr = world.get_raw(entity, ne_id);
        if ptr.is_null() {
            return false;
        }
        let net = unsafe { &*(ptr as *const NetworkEntity) };
        net.net_id == net_id
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
        self.ensure_change_observer(world);
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

            let (cold, hot) = stream_in_field_snapshot(world, entity, net);
            let prefab_id = self
                .prefab_ids
                .id_of(&prefab)
                .unwrap_or(PrefabNetId::INVALID);
            if !prefab_id.is_valid() {
                log::warn!(
                    "[prefab_rep] unknown prefab id for '{prefab}' ent={entity:#x} — skip stream.in"
                );
                return;
            }

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
                prefab_id,
                cold,
                hot,
            };
            out.push(rpc.to_packet());
        });
        out
    }

    /// Register the Flecs `OnSet` observer that marks written entities dirty.
    /// Called from the spawn emit paths (they always receive `world`); on
    /// failure `emit_hot_deltas` falls back to the full per-tick scan.
    fn ensure_change_observer(&mut self, world: &World) {
        if self.observer_checked {
            return;
        }
        self.observer_checked = true;
        self.change_tracking = world
            .register_observer(
                "_",
                &[WatchEvent::OnSet],
                std::ptr::null_mut(),
                on_set_mark_dirty,
            )
            .is_ok();
        if !self.change_tracking {
            log::warn!("[prefab_rep] OnSet observer failed — hot deltas fall back to full scan");
        }
    }

    fn emit_hot_deltas(&mut self, world: &World) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let mut batch = Vec::new();
        if self.change_tracking {
            // Only entities written since the last emit (O(dirty), not O(all)).
            let dirty = take_replication_dirty();
            for entity in dirty {
                let Some(&net_id) = self.entity_to_net.get(&entity) else {
                    continue;
                };
                self.emit_entity_delta(world, entity, net_id, ne_id, &mut batch);
            }
        } else {
            let tracked: Vec<(EntityId, u16)> =
                self.entity_to_net.iter().map(|(&e, &n)| (e, n)).collect();
            for (entity, net_id) in tracked {
                self.emit_entity_delta(world, entity, net_id, ne_id, &mut batch);
            }
        }
        if batch.is_empty() {
            Vec::new()
        } else {
            vec![StreamUpdateBatchRpc { updates: batch }.to_packet()]
        }
    }

    /// Compare one binding's hot paths against `last_hot`; push changed fields into `batch`.
    fn emit_entity_delta(
        &mut self,
        world: &World,
        entity: EntityId,
        net_id: u16,
        ne_id: u64,
        batch: &mut Vec<StreamEntityUpdate>,
    ) {
        if !self.binding_valid(world, entity, net_id, ne_id) {
            return;
        }
        let ptr = world.get_raw(entity, ne_id) as *const NetworkEntity;
        if ptr.is_null() {
            return;
        }
        let net = unsafe { &*ptr };
        if net.is_dormant() {
            // Keep the mark so the entity is re-checked once it wakes up.
            if self.change_tracking {
                mark_replication_dirty(entity);
            }
            return;
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
            return;
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

    fn emit_destroys(&mut self, world: &World) -> Vec<Vec<u8>> {
        let ne_id = world.component_init::<NetworkEntity>();
        let tracked: Vec<(EntityId, u16)> =
            self.entity_to_net.iter().map(|(&e, &n)| (e, n)).collect();
        let mut out = Vec::new();
        for (entity, net_id) in tracked {
            if self.binding_valid(world, entity, net_id, ne_id) {
                continue;
            }
            self.drop_binding(entity, net_id);
            out.push(StreamOutRpc { net_id }.to_packet());
        }
        out
    }
}

/// Flecs `OnSet` bridge: mark every written entity for the next hot-delta emit.
extern "C-unwind" fn on_set_mark_dirty(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    _ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || count == 0 {
        return;
    }
    for i in 0..count as usize {
        mark_replication_dirty(unsafe { *entities.add(i) });
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

#[cfg(test)]
mod dirty_delta_tests {
    use super::*;
    use deerpunk_engine_common::components::TransformComponent;
    use deerpunk_engine_common::network::StreamInRpc;
    use deerpunk_engine_common::property::{register_engine_layout_properties, PropertyRegistry};
    use glam::Vec3;

    /// Dirty-set hot deltas: no changes → no packets; one field write → exactly
    /// that field's delta; spawn still sends full state.
    #[test]
    fn hot_deltas_follow_dirty_marks_only() {
        let world = World::new();
        register_engine_layout_properties(&world);
        let pos = PropertyRegistry::global()
            .lock()
            .unwrap()
            .resolve_path("TransformComponent.position")
            .expect("position property registered");

        let mut rep = ServerPrefabReplication::new();
        rep.prefab_ids
            .register(PrefabNetId(1), "test_ent")
            .expect("register test prefab");

        let t_id = world.component_init::<TransformComponent>();
        let ne_id = world.component_init::<NetworkEntity>();
        let spawn_ent = |y: f32| {
            let e = world.spawn();
            world.set(
                e,
                t_id,
                TransformComponent {
                    position: Vec3::new(0.0, y, 0.0),
                    ..Default::default()
                },
            );
            let mut ne = NetworkEntity::from_paths(&[pos], &[]);
            ne.set_prefab("test_ent");
            world.set(e, ne_id, ne);
            e
        };
        let e1 = spawn_ent(1.0);
        let e2 = spawn_ent(2.0);

        // Spawn: full initial state via `stream.in` for both entities.
        let spawn_packets = rep.snapshot(&world);
        let stream_ins = spawn_packets
            .iter()
            .filter(|p| StreamInRpc::decode(&p[1..]).is_some())
            .count();
        assert_eq!(stream_ins, 2, "snapshot sends full state per entity");
        assert!(rep.change_tracking, "OnSet observer must register");

        // No writes since spawn: no delta work, dirty set fully drained.
        assert!(rep.tick(&world).is_empty(), "no changes → no packets");
        assert!(take_replication_dirty().is_empty(), "marks drained");

        // One field write on e1: exactly that entity + field in the batch.
        world.set(
            e1,
            t_id,
            TransformComponent {
                position: Vec3::new(5.0, 1.0, 0.0),
                ..Default::default()
            },
        );
        let packets = rep.tick(&world);
        assert_eq!(packets.len(), 1, "one update batch packet");
        let batch = StreamUpdateBatchRpc::decode(&packets[0][1..]).expect("decode batch");
        assert_eq!(batch.updates.len(), 1, "only the written entity");
        assert_eq!(batch.updates[0].net_id, rep.net_id_of(e1).unwrap());
        assert_eq!(batch.updates[0].fields.len(), 1, "only the changed field");
        assert_eq!(batch.updates[0].fields[0].0, pos);
        match &batch.updates[0].fields[0].1 {
            FieldValue::Vec3(v) => assert_eq!(v, &[5.0, 1.0, 0.0]),
            other => panic!("expected Vec3 delta, got {other:?}"),
        }

        // Idle again: nothing.
        assert!(rep.tick(&world).is_empty());
        let _ = e2;
    }
}
