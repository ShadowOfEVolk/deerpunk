//! Listen-server replication: server tick → loopback / UDP → client world.

use crate::Host;

impl Host {
    #[cfg(any(feature = "client", feature = "full"))]
    pub(crate) fn sync_client_world(&mut self) {
        if self.client_engine.is_none() {
            return;
        }

        let predict = self.cvar.get_usize_or("cl_predict", 1) != 0;
        let pawn = self.local_client_entity();
        if let Some(client) = self.client_engine.as_mut() {
            client.local_sim.set_local_pawn(pawn, predict);
        }

        {
            let _g = crate::system_profiler::scope("replicate.apply_s2c");
            self.drain_loopback_from_server();
        }

        self.flush_client_pending_albedo_slot();
    }

    #[cfg(not(any(feature = "client", feature = "full")))]
    pub(crate) fn sync_client_world(&mut self) {}

    #[cfg(any(feature = "client", feature = "full"))]
    pub(crate) fn drain_loopback_from_server(&mut self) {
        for packet in self.loopback.receive_server_packets() {
            self.apply_local_s2c(&packet);
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub(crate) fn flush_client_pending_albedo_slot(&mut self) {
        let Some(client) = self.client_engine.as_ref() else {
            return;
        };
        let world = &client.world as *const deerpunk_engine_common::world::World;
        let s = unsafe { self.flush_pending_albedos_world(&*world) };
        if s != 0 {
            if let Some(client) = self.client_engine.as_mut() {
                client.last_atlas_slot = s;
            }
        }
    }
}
