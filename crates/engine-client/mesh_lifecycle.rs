//! Per-entity mesh registry + GPU pool refs. Updated on `client_world:set` / destroy only.

use std::collections::HashMap;

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
use deerpunk_engine_common::world::{ComponentId, EntityId, World};
use deerpunk_engine_renderer::Renderer;

pub struct MeshGpuLifecycle {
    mesh3d: Option<ComponentId>,
    skinned: Option<ComponentId>,
    /// Last known GPU mesh content id per entity (renderer pool).
    entity_content: HashMap<EntityId, u64>,
    /// Last registry handle we attached for this entity.
    entity_mesh_handle: HashMap<EntityId, MeshHandle>,
}

impl MeshGpuLifecycle {
    pub fn new() -> Self {
        Self {
            mesh3d: None,
            skinned: None,
            entity_content: HashMap::new(),
            entity_mesh_handle: HashMap::new(),
        }
    }

    fn mesh3d_id(&mut self, world: &World) -> ComponentId {
        *self.mesh3d.get_or_insert_with(|| world.component_init::<Mesh3D>())
    }

    fn skinned_id(&mut self, world: &World) -> ComponentId {
        *self.skinned
            .get_or_insert_with(|| world.component_init::<SkinnedMesh3D>())
    }

    fn mesh_handle_on_entity(
        world: &World,
        mesh3d: ComponentId,
        skinned: ComponentId,
        entity: EntityId,
    ) -> MeshHandle {
        if world.has(entity, skinned) {
            return Self::handle_skinned(world, skinned, entity);
        }
        Self::handle_on_entity(world, mesh3d, entity)
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

    fn gpu_ids_for_entity(
        meshes: &MeshRegistry,
        world: &World,
        mesh3d: ComponentId,
        skinned: ComponentId,
        entity: EntityId,
    ) -> (u64, u32) {
        if world.has(entity, skinned) {
            let ptr = world.get_raw(entity, skinned);
            if ptr.is_null() {
                return (0, 0);
            }
            let m = unsafe { &*(ptr as *const SkinnedMesh3D) };
            let id = meshes.content_id(m.mesh);
            let alloc = meshes.get(m.mesh).map(|mesh| mesh.allocator_id).unwrap_or(0);
            return (id, alloc);
        }
        let ptr = world.get_raw(entity, mesh3d);
        if ptr.is_null() {
            return (0, 0);
        }
        let m = unsafe { &*(ptr as *const Mesh3D) };
        let id = meshes.content_id(m.mesh);
        let alloc = meshes.get(m.mesh).map(|mesh| mesh.allocator_id).unwrap_or(0);
        (id, alloc)
    }

    fn reconcile_registry_handle(
        &mut self,
        meshes: &mut MeshRegistry,
        world: &World,
        entity: EntityId,
    ) {
        let mesh3d = self.mesh3d_id(world);
        let skinned = self.skinned_id(world);
        let new_handle = Self::mesh_handle_on_entity(world, mesh3d, skinned, entity);
        let prev = self.entity_mesh_handle.get(&entity).copied();
        if prev == Some(new_handle) {
            return;
        }
        if let Some(old) = prev {
            if old.is_valid() {
                meshes.detach(old);
            }
        }
        if new_handle.is_valid() {
            meshes.attach(new_handle);
            self.entity_mesh_handle.insert(entity, new_handle);
        } else {
            self.entity_mesh_handle.remove(&entity);
        }
    }

    fn reconcile_gpu_content(
        &mut self,
        meshes: &MeshRegistry,
        world: &World,
        renderer: &mut Renderer,
        entity: EntityId,
    ) {
        let mesh3d = self.mesh3d_id(world);
        let skinned = self.skinned_id(world);
        let (new_id, allocator_id) =
            Self::gpu_ids_for_entity(meshes, world, mesh3d, skinned, entity);
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

    /// Entity mesh component installed or replaced (`client_world:set` Mesh3D / SkinnedMesh3D).
    pub fn adopt_entity(
        &mut self,
        meshes: &mut MeshRegistry,
        world: &World,
        renderer: &mut Renderer,
        entity: EntityId,
    ) {
        self.reconcile_registry_handle(meshes, world, entity);
        self.reconcile_gpu_content(meshes, world, renderer, entity);
    }

    /// Entity removed — drop registry + GPU refs for this entity only.
    pub fn release_entity(
        &mut self,
        meshes: &mut MeshRegistry,
        world: &World,
        renderer: &mut Renderer,
        entity: EntityId,
    ) {
        if let Some(h) = self.entity_mesh_handle.remove(&entity) {
            if h.is_valid() {
                meshes.detach(h);
            }
        }
        if let Some(id) = self.entity_content.remove(&entity) {
            if id != 0 {
                renderer.mesh_release(id);
            }
            return;
        }
        let mesh3d = self.mesh3d_id(world);
        let skinned = self.skinned_id(world);
        let (id, _) = Self::gpu_ids_for_entity(meshes, world, mesh3d, skinned, entity);
        if id != 0 {
            renderer.mesh_release(id);
        }
    }

    pub fn reset(&mut self, renderer: &mut Renderer) {
        self.entity_content.clear();
        self.entity_mesh_handle.clear();
        self.mesh3d = None;
        self.skinned = None;
        renderer.reset_mesh_gpu();
    }

    pub fn reset_tracking(&mut self) {
        self.entity_content.clear();
        self.entity_mesh_handle.clear();
        self.mesh3d = None;
        self.skinned = None;
    }
}
