//! Server world helpers for MethodDB (`v1:world:*`).

use std::ffi::c_void;
impl crate::Host {
    pub fn clear_server_world(&mut self) {
        let world = &self.server_engine.world;
        let ids: Vec<u64> = world.inspect_entities().into_iter().map(|e| e.id).collect();
        log::info!("[world] clear — destroying {} entities", ids.len());
        for id in ids {
            world.destroy(id);
        }
        world.simulate(0.0);
        log::debug!("[world] clear done");
    }

    pub fn world_get_component_ptr(&self, entity: u64, comp: u64) -> *mut c_void {
        let ptr = self.server_engine.world.get_raw(entity, comp);
        if ptr.is_null() {
            std::ptr::null_mut()
        } else {
            ptr as *mut c_void
        }
    }

    pub fn world_set_component_raw(
        &self,
        entity: u64,
        comp: u64,
        data: *const c_void,
        size: usize,
    ) {
        self.server_engine
            .world
            .set_raw(entity, comp, data, size);
    }

}
