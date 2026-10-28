//! Tracks which ECS entities own GPU mesh content refs (`Mesh::content_id`).

use std::collections::{HashMap, HashSet};

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
use deerpunk_engine_common::world::{ComponentId, EntityId, World};
use deerpunk_engine_renderer::Renderer;

pub struct MeshGpuLifecycle {
    mesh3d: Option<ComponentId>,
    skinned: Option<ComponentId>,
    mesh_query: Option<deerpunk_engine_common::world::QueryId>,
    skinned_query: Option<deerpunk_engine_common::world::QueryId>,
    entity_content: HashMap<EntityId, u64>,
}

impl MeshGpuLifecycle {
    pub fn new() -> Self {
        Self {
            mesh3d: None,
            skinned: None,
            mesh_query: None,
            skinned_query: None,
            entity_content: HashMap::new(),
        }
    }

    fn mesh3d_id(&mut self, world: &World) -> ComponentId {
        *self.mesh3d.get_or_insert_with(|| world.component_init::<Mesh3D>())
    }

    fn skinned_id(&mut self, world: &World) -> ComponentId {
        *self.skinned
            .get_or_insert_with(|| world.component_init::<SkinnedMesh3D>())
    }

    fn ensure_mesh_query(&mut self, world: &mut World) -> deerpunk_engine_common::world::QueryId {
        if let Some(q) = self.mesh_query {
            return q;
        }
        let q = world.query_prepare("Mesh3D");
        self.mesh_query = Some(q);
        q
    }

    fn ensure_skinned_query(
        &mut self,
        world: &mut World,
    ) -> deerpunk_engine_common::world::QueryId {
        if let Some(q) = self.skinned_query {
            return q;
        }
        let q = world.query_prepare("SkinnedMesh3D");
        self.skinned_query = Some(q);
        q
    }

    fn handle_on_entity(world: &World, comp: ComponentId, entity: EntityId) -> MeshHandle {
        let ptr = world.get_raw(entity, comp);
        if ptr.is_null() {
            return MeshHandle::INVALID;
        }
        let m = unsafe { &*(ptr as *const Mesh3D) };
        m.mesh
    }

    fn handle_skinned(world: &World, comp: ComponentId, entity: EntityId) -> MeshHandle {
        let ptr = world.get_raw(entity, comp);
        if ptr.is_null() {
            return MeshHandle::INVALID;
        }
        let m = unsafe { &*(ptr as *const SkinnedMesh3D) };
        m.mesh
    }

    fn allocator_id_skinned(
        meshes: &MeshRegistry,
        world: &World,
        comp: ComponentId,
        entity: EntityId,
    ) -> u32 {
        let ptr = world.get_raw(entity, comp);
        if ptr.is_null() {
            return 0;
        }
        let m = unsafe { &*(ptr as *const SkinnedMesh3D) };
        meshes
            .get(m.mesh)
            .map(|mesh| mesh.allocator_id)
            .unwrap_or(0)
    }

    fn content_id_skinned(
        meshes: &MeshRegistry,
        world: &World,
        comp: ComponentId,
        entity: EntityId,
    ) -> u64 {
        let ptr = world.get_raw(entity, comp);
        if ptr.is_null() {
            return 0;
        }
        let m = unsafe { &*(ptr as *const SkinnedMesh3D) };
        meshes.content_id(m.mesh)
    }

    fn allocator_id_on_entity(
        meshes: &MeshRegistry,
        world: &World,
        mesh3d: ComponentId,
        entity: EntityId,
    ) -> u32 {
        let ptr = world.get_raw(entity, mesh3d);
        if ptr.is_null() {
            return 0;
        }
        let m = unsafe { &*(ptr as *const Mesh3D) };
        meshes
            .get(m.mesh)
            .map(|mesh| mesh.allocator_id)
            .unwrap_or(0)
    }

    fn content_id_on_entity(
        meshes: &MeshRegistry,
        world: &World,
        mesh3d: ComponentId,
        entity: EntityId,
    ) -> u64 {
        let ptr = world.get_raw(entity, mesh3d);
        if ptr.is_null() {
            return 0;
        }
        let m = unsafe { &*(ptr as *const Mesh3D) };
        meshes.content_id(m.mesh)
    }

    /// Entity now has (or changed) `Mesh3D` — bump GPU refcount for its content id.
    pub fn adopt_entity(
        &mut self,
        meshes: &MeshRegistry,
        world: &World,
        renderer: &mut Renderer,
        entity: EntityId,
    ) {
        let mesh3d = self.mesh3d_id(world);
        let new_id = Self::content_id_on_entity(meshes, world, mesh3d, entity);
        let allocator_id = Self::allocator_id_on_entity(meshes, world, mesh3d, entity);
        let old = self.entity_content.get(&entity).copied().unwrap_or(0);
        if old == new_id {
            return;
        }
        if old != 0 {
            renderer.mesh_release(old);
        }
        if new_id != 0 {
            renderer.mesh_retain(new_id, allocator_id);
            self.entity_content.insert(entity, new_id);
        } else {
            self.entity_content.remove(&entity);
        }
    }

    /// Entity removed from the world — release its mesh content ref.
    pub fn release_entity(
        &mut self,
        meshes: &MeshRegistry,
        world: &World,
        renderer: &mut Renderer,
        entity: EntityId,
    ) {
        if let Some(id) = self.entity_content.remove(&entity) {
            if id != 0 {
                renderer.mesh_release(id);
            }
            return;
        }
        let mesh3d = self.mesh3d_id(world);
        let skinned = self.skinned_id(world);
        let id = Self::content_id_on_entity(meshes, world, mesh3d, entity);
        if id != 0 {
            renderer.mesh_release(id);
            return;
        }
        let id = Self::content_id_skinned(meshes, world, skinned, entity);
        if id != 0 {
            renderer.mesh_release(id);
        }
    }

    pub fn reset(&mut self, renderer: &mut Renderer) {
        self.entity_content.clear();
        self.mesh3d = None;
        self.skinned = None;
        self.mesh_query = None;
        self.skinned_query = None;
        renderer.reset_mesh_gpu();
    }

    /// Drop cached queries / entity→mesh map without touching the GPU pool (caller resets renderer).
    pub fn reset_tracking(&mut self) {
        self.entity_content.clear();
        self.mesh3d = None;
        self.skinned = None;
        self.mesh_query = None;
        self.skinned_query = None;
    }

    /// Reconcile GPU refs with entities that currently have `Mesh3D`.
    fn reconcile_entity(
        &mut self,
        ent: EntityId,
        new_id: u64,
        allocator_id: u32,
        renderer: &mut Renderer,
    ) {
        let old = self.entity_content.get(&ent).copied().unwrap_or(0);
        if old != new_id {
            if old != 0 {
                renderer.mesh_release(old);
            }
            if new_id != 0 {
                renderer.mesh_retain(new_id, allocator_id);
                self.entity_content.insert(ent, new_id);
            } else {
                self.entity_content.remove(&ent);
            }
        } else if new_id != 0 && !self.entity_content.contains_key(&ent) {
            renderer.mesh_retain(new_id, allocator_id);
            self.entity_content.insert(ent, new_id);
        }
    }

    pub fn sync_from_world(
        &mut self,
        meshes: &mut MeshRegistry,
        world: &mut World,
        renderer: &mut Renderer,
    ) {
        let q = self.ensure_mesh_query(world);
        let sq = self.ensure_skinned_query(world);
        let mesh3d = self.mesh3d_id(world);
        let skinned = self.skinned_id(world);
        let mut live_entities = HashSet::new();
        let mut live_handles: HashSet<usize> = HashSet::new();
        if let Some(batches) = world.query_batches(q) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                for &ent in entities {
                    live_entities.insert(ent);
                    let h = Self::handle_on_entity(world, mesh3d, ent);
                    if h.is_valid() {
                        live_handles.insert(h.0);
                    }
                    let new_id = Self::content_id_on_entity(meshes, world, mesh3d, ent);
                    let allocator_id = Self::allocator_id_on_entity(meshes, world, mesh3d, ent);
                    self.reconcile_entity(ent, new_id, allocator_id, renderer);
                }
            }
        }
        if let Some(batches) = world.query_batches(sq) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                for &ent in entities {
                    live_entities.insert(ent);
                    let h = Self::handle_skinned(world, skinned, ent);
                    if h.is_valid() {
                        live_handles.insert(h.0);
                    }
                    let new_id = Self::content_id_skinned(meshes, world, skinned, ent);
                    let allocator_id = Self::allocator_id_skinned(meshes, world, skinned, ent);
                    self.reconcile_entity(ent, new_id, allocator_id, renderer);
                }
            }
        }
        for ent in self
            .entity_content
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            if !live_entities.contains(&ent) {
                self.release_entity(meshes, world, renderer, ent);
            }
        }
        // Reclaim registry slots that no entity references. Must run before
        // RenderExtractor::extract so the renderer never sees a stale handle.
        meshes.sweep(&live_handles);
    }
}
