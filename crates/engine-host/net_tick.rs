//! Poll [`ServerNet`] / [`ClientNet`] and LAN.

use deerpunk_engine_common::network::ClientSessionPhase;

use crate::Host;

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

        if self.client_engine.as_ref().is_some_and(|c| c.network.is_active())
            && self.client_engine.as_ref().is_some_and(|c| c.network.is_connected())
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

    /// Server simulation thread: listen socket + LAN + C2S.
    pub(crate) fn tick_net_server(&mut self, dt: f32) {
        if self.server_engine.is_none() {
            return;
        }
        self.drain_pending_net_cmds();
        self.tick_lan();
        let slots = self.server_mut().players.slots();
        let duration = std::time::Duration::from_secs_f32(dt.max(0.0));
        let lan_reply = if self.server_mut().network.is_listening() {
            Some(self.lan_advertise_state())
        } else {
            None
        };
        let (c2s, need_public) = self.server_mut().network.poll(
            duration,
            &slots,
            lan_reply.as_ref(),
        );
        if let Some(addr) = need_public {
            self.server_mut().network.apply_need_public_addr(addr);
        }

        let udp_connects = std::mem::take(&mut self.server_mut().network.pending_player_connect);
        for &slot in &udp_connects {
            self.server_mut().on_udp_player_connected(slot);
            if let Some(ref dll) = self.server_mut().dll {
                unsafe {
                    dll.player_connected(slot);
                }
            }
        }
        if !udp_connects.is_empty() {
            self.finish_client_join(&udp_connects);
        }

        let disconnects = std::mem::take(&mut self.server_mut().network.pending_player_disconnect);
        for slot in disconnects {
            if let Some(ref dll) = self.server_mut().dll {
                unsafe {
                    dll.player_disconnected(slot);
                }
            }
            self.server_mut().on_udp_player_disconnected(slot);
        }

        for (slot, packet) in c2s {
            self.parse_client_message(slot, &packet);
        }
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
