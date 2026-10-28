//! Server UDP socket (bind + renet host). Game wire = `CLC` / `SVC` bytes.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, SystemTime};

use deerpunk_engine_common::network::lan::{LanAdvertiseState, DEFAULT_GAME_PORT};
use deerpunk_engine_common::network::protocol::SVC;
use renet::{ClientId, ConnectionConfig, DefaultChannel, RenetServer, ServerEvent};
use renet_netcode::{ServerAuthentication, ServerConfig, NETCODE_KEY_BYTES};

use crate::connect_auth;
use crate::net::NetHudStats;
use crate::udp_demux::DemuxServerTransport;

pub const PROTOCOL_ID: u64 = 0x4445_4552_5055_4e4b;
pub const DEFAULT_PORT: u16 = DEFAULT_GAME_PORT;

fn channel_for_svc(cmd: u8) -> DefaultChannel {
    if cmd == SVC::NetDelta as u8 {
        DefaultChannel::Unreliable
    } else {
        DefaultChannel::ReliableOrdered
    }
}

struct HostSession {
    server: RenetServer,
    transport: DemuxServerTransport,
    client_slots: HashMap<ClientId, u32>,
    private_key: [u8; NETCODE_KEY_BYTES],
    public_addresses: Vec<SocketAddr>,
    listen_port: u16,
}

/// Listen / dedicated: one bound UDP socket, remote clients over renet.
#[derive(Default)]
pub struct ServerNet {
    host: Option<HostSession>,
    pub pending_player_connect: Vec<u32>,
    pub pending_player_disconnect: Vec<u32>,
    public_address_extra: Vec<SocketAddr>,
    last_listen_bind: Option<SocketAddr>,
    last_private_key: Option<[u8; NETCODE_KEY_BYTES]>,
}

impl ServerNet {
    pub fn is_listening(&self) -> bool {
        self.host.is_some()
    }

    pub fn disconnect(&mut self) {
        if let Some(h) = &mut self.host {
            h.transport.disconnect_all(&mut h.server);
        }
        self.host = None;
        self.pending_player_connect.clear();
        self.pending_player_disconnect.clear();
        log::info!("[net] server disconnected");
    }

    fn merged_public_addresses(&self, bind: SocketAddr) -> Vec<SocketAddr> {
        let mut v = connect_auth::listen_public_addresses(bind);
        for a in &self.public_address_extra {
            if !v.contains(a) {
                v.push(*a);
            }
        }
        v
    }

    pub fn listen(
        &mut self,
        addr: SocketAddr,
        private_key: [u8; NETCODE_KEY_BYTES],
    ) -> Result<(), String> {
        self.disconnect();
        let port = addr.port();
        let public_addresses = self.merged_public_addresses(addr);
        self.last_listen_bind = Some(addr);
        self.last_private_key = Some(private_key);
        let socket = deerpunk_engine_common::network::lan::bind_reuse_broadcast(port)?;
        let current_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let server_config = ServerConfig {
            current_time,
            max_clients: 16,
            protocol_id: PROTOCOL_ID,
            public_addresses: public_addresses.clone(),
            authentication: ServerAuthentication::Secure { private_key },
        };
        let transport = DemuxServerTransport::new(server_config, socket, port)
            .map_err(|e| format!("DemuxServerTransport: {e}"))?;
        let server = RenetServer::new(ConnectionConfig::default());
        log::info!(
            "[net] listening on {addr} protocol={PROTOCOL_ID:#x} public={:?}",
            public_addresses
        );
        self.host = Some(HostSession {
            server,
            transport,
            client_slots: HashMap::new(),
            private_key,
            public_addresses,
            listen_port: port,
        });
        Ok(())
    }

    pub fn apply_need_public_addr(&mut self, addr: SocketAddr) {
        if self.public_address_extra.contains(&addr) {
            return;
        }
        self.public_address_extra.push(addr);
        if let (Some(bind), Some(key)) = (self.last_listen_bind, self.last_private_key) {
            log::info!("[connect_auth] re-listen with public addr {addr}");
            let _ = self.listen(bind, key);
        }
    }

    pub fn listen_port(&self) -> Option<u16> {
        self.host
            .as_ref()
            .and_then(|h| h.transport.addresses().first().map(|a| a.port()))
    }

    pub fn poll(
        &mut self,
        dt: Duration,
        existing_slots: &[u32],
        lan_reply: Option<&LanAdvertiseState>,
    ) -> (Vec<(u32, Vec<u8>)>, Option<SocketAddr>) {
        let mut inbound_c2s = Vec::new();
        let mut need_public = None;
        let Some(h) = &mut self.host else {
            return (inbound_c2s, need_public);
        };

        h.server.update(dt);
        match h.transport.update(
            dt,
            &mut h.server,
            &h.private_key,
            &h.public_addresses,
            lan_reply,
        ) {
            Ok(n) => need_public = n,
            Err(e) => log::warn!("[net] server transport.update: {e}"),
        }

        let mut connects = Vec::new();
        let mut disconnects = Vec::new();
        while let Some(event) = h.server.get_event() {
            match event {
                ServerEvent::ClientConnected { client_id } => connects.push(client_id),
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    disconnects.push((client_id, reason));
                }
            }
        }
        for client_id in connects {
            let mut used: Vec<u32> = existing_slots.to_vec();
            used.extend(h.client_slots.values().copied());
            let slot = Self::alloc_slot_simple(&used);
            h.client_slots.insert(client_id, slot);
            self.pending_player_connect.push(slot);
            log::info!("[net] remote client {client_id} → slot={slot}");
        }
        for (client_id, reason) in disconnects {
            if let Some(slot) = h.client_slots.remove(&client_id) {
                self.pending_player_disconnect.push(slot);
                log::info!(
                    "[net] remote client {client_id} disconnected slot={slot}: {reason}"
                );
            }
        }

        for client_id in h.server.clients_id() {
            let Some(&slot) = h.client_slots.get(&client_id) else {
                continue;
            };
            while let Some(msg) = h.server.receive_message(client_id, DefaultChannel::Unreliable) {
                inbound_c2s.push((slot, msg.to_vec()));
            }
            while let Some(msg) = h.server.receive_message(client_id, DefaultChannel::ReliableOrdered) {
                inbound_c2s.push((slot, msg.to_vec()));
            }
        }

        h.transport.send_packets(&mut h.server);
        (inbound_c2s, need_public)
    }

    fn alloc_slot_simple(used: &[u32]) -> u32 {
        let mut slot = 0u32;
        while used.contains(&slot) {
            slot += 1;
        }
        slot
    }

    pub fn broadcast_s2c(&mut self, packets: &[Vec<u8>]) {
        let Some(h) = &mut self.host else {
            return;
        };
        for packet in packets {
            let Some(&cmd) = packet.first() else {
                continue;
            };
            h.server.broadcast_message(channel_for_svc(cmd), packet.clone());
        }
    }

    pub fn send_s2c_to_slot(&mut self, slot: u32, packets: &[Vec<u8>]) {
        let Some(h) = &mut self.host else {
            return;
        };
        let Some((&client_id, _)) = h.client_slots.iter().find(|(_, s)| **s == slot) else {
            return;
        };
        for packet in packets {
            let Some(&cmd) = packet.first() else {
                continue;
            };
            h.server.send_message(client_id, channel_for_svc(cmd), packet.clone());
        }
    }

    pub fn hud_stats(&self) -> NetHudStats {
        let Some(h) = &self.host else {
            return NetHudStats {
                role: "idle",
                connected: false,
                rtt_ms: 0.0,
                packet_loss: 0.0,
                bytes_in_per_sec: 0.0,
                bytes_out_per_sec: 0.0,
                remote_clients: 0,
            };
        };
        let ids = h.server.clients_id();
        let mut rtt = 0.0f64;
        let mut loss = 0.0f64;
        let mut bin = 0.0f64;
        let mut bout = 0.0f64;
        let mut n = 0u32;
        for id in ids {
            if let Ok(info) = h.server.network_info(id) {
                rtt += info.rtt;
                loss += info.packet_loss;
                bin += info.bytes_received_per_second;
                bout += info.bytes_sent_per_second;
                n += 1;
            }
        }
        if n > 0 {
            rtt /= n as f64;
            loss /= n as f64;
        }
        NetHudStats {
            role: "host",
            connected: true,
            rtt_ms: (rtt * 1000.0) as f32,
            packet_loss: loss as f32,
            bytes_in_per_sec: bin,
            bytes_out_per_sec: bout,
            remote_clients: n,
        }
    }

    pub fn remote_slot_debug(&self) -> Vec<(u32, String)> {
        let Some(h) = &self.host else {
            return Vec::new();
        };
        h.client_slots
            .iter()
            .map(|(cid, slot)| {
                let addr = h
                    .transport
                    .client_addr(*cid)
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "?".into());
                (*slot, addr)
            })
            .collect()
    }
}

pub fn is_connect_to_local_listen(addr: SocketAddr, listen_port: u16) -> bool {
    if addr.port() != listen_port {
        return false;
    }
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

pub fn parse_addr(s: &str, default_port: u16) -> Result<SocketAddr, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty address".into());
    }
    if let Ok(addr) = s.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Some((host, port)) = s.rsplit_once(':') {
        let port: u16 = port.parse().map_err(|_| format!("bad port in '{s}'"))?;
        format!("{host}:{port}")
            .parse()
            .map_err(|e| format!("bad addr '{s}': {e}"))
    } else {
        format!("{s}:{default_port}")
            .parse()
            .map_err(|e| format!("bad addr '{s}': {e}"))
    }
}
