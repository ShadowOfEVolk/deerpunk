//! Tracks which ECS entities own GPU mesh content refs (`Mesh::content_id`).

use std::collections::{HashMap, HashSet};

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::world::{ComponentId, EntityId, World};
use deerpunk_engine_renderer::Renderer;

pub struct MeshGpuLifecycle {
    mesh3d: Option<ComponentId>,
    mesh_query: Option<deerpunk_engine_common::world::QueryId>,
    entity_content: HashMap<EntityId, u64>,
}

impl MeshGpuLifecycle {
    pub fn new() -> Self {
        Self {
            mesh3d: None,
            mesh_query: None,
            entity_content: HashMap::new(),
        }
    }

    fn mesh3d_id(&mut self, world: &World) -> ComponentId {
        *self.mesh3d.get_or_insert_with(|| world.component_init::<Mesh3D>())
    }

    fn ensure_mesh_query(&mut self, world: &mut World) -> deerpunk_engine_common::world::QueryId {
        if let Some(q) = self.mesh_query {
            return q;
        }
        let q = world.query_prepare("Mesh3D");
        self.mesh_query = Some(q);
        q
    }

    fn content_id_on_entity(world: &World, mesh3d: ComponentId, entity: EntityId) -> u64 {
        let ptr = world.get_raw(entity, mesh3d);
        if ptr.is_null() {
            return 0;
        }
        let m = unsafe { &*(ptr as *const Mesh3D) };
        m.mesh.as_ref().map(|arc| arc.content_id()).unwrap_or(0)
    }

    /// Entity now has (or changed) `Mesh3D` — bump GPU refcount for its content id.
    pub fn adopt_entity(&mut self, world: &World, renderer: &mut Renderer, entity: EntityId) {
        let mesh3d = self.mesh3d_id(world);
        let new_id = Self::content_id_on_entity(world, mesh3d, entity);
        let old = self.entity_content.get(&entity).copied().unwrap_or(0);
        if old == new_id {
            return;
        }
        if old != 0 {
            renderer.mesh_release(old);
        }
        if new_id != 0 {
            renderer.mesh_retain(new_id);
            self.entity_content.insert(entity, new_id);
        } else {
            self.entity_content.remove(&entity);
        }
    }

    /// Entity removed from the world — release its mesh content ref.
    pub fn release_entity(&mut self, world: &World, renderer: &mut Renderer, entity: EntityId) {
        let mesh3d = self.mesh3d_id(world);
        let id = self
            .entity_content
            .remove(&entity)
            .unwrap_or_else(|| Self::content_id_on_entity(world, mesh3d, entity));
        if id != 0 {
            renderer.mesh_release(id);
        }
    }

    pub fn reset(&mut self, renderer: &mut Renderer) {
        self.entity_content.clear();
        self.mesh3d = None;
        self.mesh_query = None;
        renderer.reset_mesh_gpu();
    }

    /// Drop cached queries / entity→mesh map without touching the GPU pool (caller resets renderer).
    pub fn reset_tracking(&mut self) {
        self.entity_content.clear();
        self.mesh3d = None;
        self.mesh_query = None;
    }

    /// Reconcile GPU refs with entities that currently have `Mesh3D`.
    pub fn sync_from_world(&mut self, world: &mut World, renderer: &mut Renderer) {
        let q = self.ensure_mesh_query(world);
        let mesh3d = self.mesh3d_id(world);
        let mut live = HashSet::new();
        if let Some(batches) = world.query_batches(q) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                for &ent in entities {
                    live.insert(ent);
                    let new_id = Self::content_id_on_entity(world, mesh3d, ent);
                    let old = self.entity_content.get(&ent).copied().unwrap_or(0);
                    if old != new_id {
                        if old != 0 {
                            renderer.mesh_release(old);
                        }
                        if new_id != 0 {
                            renderer.mesh_retain(new_id);
                            self.entity_content.insert(ent, new_id);
                        } else {
                            self.entity_content.remove(&ent);
                        }
                    } else if new_id != 0 && !self.entity_content.contains_key(&ent) {
                        renderer.mesh_retain(new_id);
                        self.entity_content.insert(ent, new_id);
                    }
                }
            }
        }
        for ent in self
            .entity_content
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            if !live.contains(&ent) {
                self.release_entity(world, renderer, ent);
            }
        }
    }
}
