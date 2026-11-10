//! Poll [`ServerNet`] / [`ClientNet`] and LAN.

use deerpunk_engine_common::network::ClientSessionPhase;

use crate::Host;

deerpunk_engine_common::cvar_def!(
    "sv_tickrate",
    "60",
    "Server simulation ticks per second (fixed timestep).",
    deerpunk_engine_common::cvar_def::ARCHIVE
);

deerpunk_engine_common::cvar_def!(
    "sv_thread_profile",
    "1",
    "Log server ticks slower than 1.5x sv_tickrate budget (default on, throttled to ~5s).",
    deerpunk_engine_common::cvar_def::ARCHIVE
);

impl Host {
    /// Client render thread: remote UDP client only.
    #[cfg(any(feature = "client", feature = "full"))]
    pub(crate) fn tick_net_client(&mut self, dt: f32) {
        // Net console queue: only the main client host may touch UDP client / forward to server thread.
        #[cfg(any(feature = "client", feature = "full"))]
        if self.client_engine.is_some() {
            self.drain_pending_net_cmds();
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        self.drain_pending_net_cmds();
        let duration = std::time::Duration::from_secs_f32(dt.max(0.0));
        let s2c = self
            .client_engine
            .as_mut()
            .map(|c| c.network.poll(duration))
            .unwrap_or_default();

        if self
            .client_engine
            .as_ref()
            .is_some_and(|c| c.network.is_active())
            && self
                .client_engine
                .as_ref()
                .is_some_and(|c| c.network.is_connected())
        {
            if let Some(client) = self.client_engine.as_mut() {
                if client.join_phase == ClientSessionPhase::Idle {
                    client.join_phase = ClientSessionPhase::Init;
                    log::info!("[net] transport up → waiting init");
                }
            }
        }

        self.apply_remote_udp_s2c(&s2c);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn apply_remote_udp_s2c(&mut self, s2c: &[Vec<u8>]) {
        if s2c.is_empty() {
            return;
        }
        let Some(client) = self.client_engine.as_ref() else {
            return;
        };
        if !client.network.is_active() {
            return;
        }
        if client.demo_player.is_some() {
            return;
        }
        for packet in s2c {
            self.apply_local_s2c(packet);
        }
        self.flush_client_pending_albedo_slot();
    }
}
