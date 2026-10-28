//! Server world helpers for MethodDB (`v1:world:*`).

use std::ffi::c_void;
impl crate::Host {
    pub fn clear_server_world(&mut self) {
        let ids: Vec<u64> = self
            .server_engine
            .as_ref()
            .expect("server thread")
            .world
            .inspect_entities()
            .into_iter()
            .map(|e| e.id)
            .collect();
        log::info!("[world] clear — destroying {} entities", ids.len());
        let world = &mut self.server_engine.as_mut().expect("server thread").world;
        for id in ids {
            world.destroy(id);
        }
        world.simulate(0.0);
        log::debug!("[world] clear done");
    }

    pub fn world_get_component_ptr(&mut self, entity: u64, comp: u64) -> *mut c_void {
        let ptr = self.server_mut().world.get_raw(entity, comp);
        if ptr.is_null() {
            std::ptr::null_mut()
        } else {
            ptr as *mut c_void
        }
    }

    pub fn world_set_component_raw(
        &mut self,
        entity: u64,
        comp: u64,
        data: *const c_void,
        size: usize,
    ) {
        if size == std::mem::size_of::<deerpunk_engine_common::components::mesh3d::Mesh3D>() {
            let mesh_registry = &mut self.mesh_registry;
            let world = &mut self.server_engine.as_mut().expect("server thread").world;
            crate::mesh_components::set_mesh3d_raw(
                world,
                mesh_registry,
                entity,
                comp,
                data,
                size,
            );
            return;
        }
        if size == std::mem::size_of::<deerpunk_engine_common::components::SkinnedMesh3D>() {
            let mesh_registry = &mut self.mesh_registry;
            let world = &mut self.server_engine.as_mut().expect("server thread").world;
            crate::mesh_components::set_skinned_raw(
                world,
                mesh_registry,
                entity,
                comp,
                data,
                size,
            );
            return;
        }
        if size == std::mem::size_of::<deerpunk_engine_common::components::MapKv>() {
            let value = unsafe { std::ptr::read(data as *const deerpunk_engine_common::components::MapKv) };
            self.server_mut().world.set(entity, comp, value);
            return;
        }
        self.server_mut()
            .world
            .set_raw(entity, comp, data, size);
    }

}
