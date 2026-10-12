//! UDP multiplayer transport (renet + netcode).
//!
//! Wire payload = same SVC/CLC packets as the in-process [`crate::channel::Channel`].
//! One UDP socket per role: OOB demux → netcode (`udp_demux`).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, SystemTime};

use deerpunk_engine_common::network::protocol::{CLC, SVC};
use renet::{ClientId, ConnectionConfig, DefaultChannel, RenetClient, RenetServer, ServerEvent};
use renet_netcode::{
    ClientAuthentication, ServerAuthentication, ServerConfig, NETCODE_KEY_BYTES,
};

use crate::connect_auth;
use crate::lan::LanAdvertiseState;
use crate::udp_demux::{DemuxClientTransport, DemuxServerTransport};

/// Must match on host and client.
pub const PROTOCOL_ID: u64 = 0x4445_4552_5055_4e4b; // "DEERPUNK"
pub const DEFAULT_PORT: u16 = 28015;

fn channel_for_svc(cmd: u8) -> DefaultChannel {
    if cmd == SVC::NetDelta as u8 {
        DefaultChannel::Unreliable
    } else {
        DefaultChannel::ReliableOrdered
    }
}

struct ServerSide {
    server: RenetServer,
    transport: DemuxServerTransport,
    /// renet ClientId → player slot
    client_slots: HashMap<ClientId, u32>,
    private_key: [u8; NETCODE_KEY_BYTES],
    public_addresses: Vec<SocketAddr>,
    listen_port: u16,
}

struct ClientSide {
    client: RenetClient,
    transport: DemuxClientTransport,
}

pub enum NetRole {
    Idle,
    Host(ServerSide),
    Client(ClientSide),
}

impl Default for NetRole {
    fn default() -> Self {
        Self::Idle
    }
}

pub struct NetTransport {
    role: NetRole,
    /// Pending remote player connects (slot already reserved).
    pub pending_player_connect: Vec<u32>,
    pub pending_player_disconnect: Vec<u32>,
    /// Extra client-facing addresses (LAN IP, etc.) merged into netcode public list.
    public_address_extra: Vec<SocketAddr>,
    last_listen_bind: Option<SocketAddr>,
    last_private_key: Option<[u8; NETCODE_KEY_BYTES]>,
}

impl Default for NetTransport {
    fn default() -> Self {
        Self {
            role: NetRole::Idle,
            pending_player_connect: Vec::new(),
            pending_player_disconnect: Vec::new(),
            public_address_extra: Vec::new(),
            last_listen_bind: None,
            last_private_key: None,
        }
    }
}

impl NetTransport {
    pub fn is_hosting(&self) -> bool {
        matches!(self.role, NetRole::Host(_))
    }

    pub fn is_client(&self) -> bool {
        matches!(self.role, NetRole::Client(_))
    }

    pub fn disconnect(&mut self) {
        match &mut self.role {
            NetRole::Host(h) => {
                h.transport.disconnect_all(&mut h.server);
            }
            NetRole::Client(c) => {
                c.transport.disconnect();
            }
            NetRole::Idle => {}
        }
        self.role = NetRole::Idle;
        self.pending_player_connect.clear();
        self.pending_player_disconnect.clear();
        log::info!("[net] disconnected");
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
        let socket = crate::lan::bind_reuse_broadcast(port)?;
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
            "[net] listening on {addr} protocol={PROTOCOL_ID:#x} secure public={:?} (single UDP demux)",
            public_addresses
        );
        self.role = NetRole::Host(ServerSide {
            server,
            transport,
            client_slots: HashMap::new(),
            private_key,
            public_addresses,
            listen_port: port,
        });
        Ok(())
    }

    /// Apply `need_public` from [`Self::update`] (re-listen with extra netcode public addr).
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
        match &self.role {
            NetRole::Host(h) => h.transport.addresses().first().map(|a| a.port()),
            _ => None,
        }
    }

    pub fn connect(&mut self, server_addr: SocketAddr) -> Result<(), String> {
        self.disconnect();
        let bind: SocketAddr = if server_addr.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };
        let socket = UdpSocket::bind(bind).map_err(|e| format!("bind: {e}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| format!("nonblocking: {e}"))?;
        let current_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let client_id = current_time.as_millis() as u64;
        log::info!("[net] connect challenge → {server_addr} client_id={client_id}");
        let connect_token =
            connect_auth::fetch_connect_token_on_socket(&socket, server_addr, client_id)?;
        let authentication = ClientAuthentication::Secure { connect_token };
        let transport = DemuxClientTransport::new(current_time, authentication, socket)
            .map_err(|e| format!("DemuxClientTransport: {e}"))?;
        let client = RenetClient::new(ConnectionConfig::default());
        self.role = NetRole::Client(ClientSide { client, transport });
        log::info!("[net] secure handshake to {server_addr}");
        Ok(())
    }

    pub fn alloc_slot(existing: &[u32]) -> u32 {
        let mut slot = 0u32;
        loop {
            if !existing.contains(&slot) {
                return slot;
            }
            slot += 1;
        }
    }

    pub fn update(
        &mut self,
        dt: Duration,
        existing_slots: &[u32],
        lan_reply: Option<&LanAdvertiseState>,
    ) -> (Vec<Vec<u8>>, Vec<(u32, Vec<u8>)>, Option<SocketAddr>) {
        let mut inbound_s2c = Vec::new();
        let mut inbound_c2s = Vec::new();
        let mut new_connect_slots = Vec::new();
        let mut new_disconnect_slots = Vec::new();
        let mut need_public = None;

        match &mut self.role {
            NetRole::Idle => {}
            NetRole::Host(h) => {
                h.server.update(dt);
                match h.transport.update(
                    dt,
                    &mut h.server,
                    &h.private_key,
                    &h.public_addresses,
                    lan_reply,
                ) {
                    Ok(n) => need_public = n,
                    Err(e) => log::warn!("[net] host transport.update: {e}"),
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
                    let slot = Self::alloc_slot(&used);
                    h.client_slots.insert(client_id, slot);
                    new_connect_slots.push(slot);
                    log::info!("[net] remote client {client_id} → slot={slot}");
                }
                for (client_id, reason) in disconnects {
                    if let Some(slot) = h.client_slots.remove(&client_id) {
                        new_disconnect_slots.push(slot);
                        log::info!(
                            "[net] remote client {client_id} disconnected slot={slot}: {reason}"
                        );
                    }
                }

                for client_id in h.server.clients_id() {
                    let Some(&slot) = h.client_slots.get(&client_id) else {
                        continue;
                    };
                    while let Some(msg) =
                        h.server.receive_message(client_id, DefaultChannel::Unreliable)
                    {
                        inbound_c2s.push((slot, msg.to_vec()));
                    }
                    while let Some(msg) = h
                        .server
                        .receive_message(client_id, DefaultChannel::ReliableOrdered)
                    {
                        inbound_c2s.push((slot, msg.to_vec()));
                    }
                }

                h.transport.send_packets(&mut h.server);
            }
            NetRole::Client(c) => {
                c.client.update(dt);
                if let Err(e) = c.transport.update(dt, &mut c.client) {
                    log::warn!("[net] client transport.update: {e}");
                }
                if c.client.is_connected() {
                    while let Some(msg) =
                        c.client.receive_message(DefaultChannel::ReliableOrdered)
                    {
                        inbound_s2c.push(msg.to_vec());
                    }
                    while let Some(msg) = c.client.receive_message(DefaultChannel::Unreliable) {
                        inbound_s2c.push(msg.to_vec());
                    }
                }
                if let Err(e) = c.transport.send_packets(&mut c.client) {
                    log::warn!("[net] client send_packets: {e}");
                }
            }
        }

        self.pending_player_connect.extend(new_connect_slots);
        self.pending_player_disconnect
            .extend(new_disconnect_slots);

        (inbound_s2c, inbound_c2s, need_public)
    }

    pub fn broadcast_s2c(&mut self, packets: &[Vec<u8>]) {
        let NetRole::Host(h) = &mut self.role else {
            return;
        };
        for packet in packets {
            let Some(&cmd) = packet.first() else {
                continue;
            };
            let ch = channel_for_svc(cmd);
            h.server.broadcast_message(ch, packet.clone());
        }
    }

    pub fn send_s2c_to_slot(&mut self, slot: u32, packets: &[Vec<u8>]) {
        let NetRole::Host(h) = &mut self.role else {
            return;
        };
        let Some((&client_id, _)) = h.client_slots.iter().find(|(_, s)| **s == slot) else {
            return;
        };
        for packet in packets {
            let Some(&cmd) = packet.first() else {
                continue;
            };
            let ch = channel_for_svc(cmd);
            h.server.send_message(client_id, ch, packet.clone());
        }
    }

    pub fn send_c2s(&mut self, packet: Vec<u8>) {
        let NetRole::Client(c) = &mut self.role else {
            return;
        };
        if !c.client.is_connected() {
            return;
        }
        let cmd = packet.first().copied().unwrap_or(0);
        let ch = if cmd == CLC::UserCmd as u8 {
            DefaultChannel::Unreliable
        } else {
            DefaultChannel::ReliableOrdered
        };
        c.client.send_message(ch, packet);
    }

    pub fn client_connected(&self) -> bool {
        match &self.role {
            NetRole::Client(c) => c.client.is_connected(),
            _ => false,
        }
    }

    pub fn hud_stats(&self) -> NetHudStats {
        match &self.role {
            NetRole::Idle => NetHudStats {
                role: "idle",
                connected: false,
                rtt_ms: 0.0,
                packet_loss: 0.0,
                bytes_in_per_sec: 0.0,
                bytes_out_per_sec: 0.0,
                remote_clients: 0,
            },
            NetRole::Host(h) => {
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
            NetRole::Client(c) => {
                let connected = c.client.is_connected();
                let info = c.client.network_info();
                NetHudStats {
                    role: if connected { "client" } else { "connecting" },
                    connected,
                    rtt_ms: (info.rtt * 1000.0) as f32,
                    packet_loss: info.packet_loss as f32,
                    bytes_in_per_sec: info.bytes_received_per_second,
                    bytes_out_per_sec: info.bytes_sent_per_second,
                    remote_clients: 0,
                }
            }
        }
    }

    pub fn remote_slot_debug(&self) -> Vec<(u32, String)> {
        match &self.role {
            NetRole::Host(h) => h
                .client_slots
                .iter()
                .map(|(cid, slot)| {
                    let addr = h
                        .transport
                        .client_addr(*cid)
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| "?".into());
                    (*slot, addr)
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NetHudStats {
    pub role: &'static str,
    pub connected: bool,
    pub rtt_ms: f32,
    pub packet_loss: f32,
    pub bytes_in_per_sec: f64,
    pub bytes_out_per_sec: f64,
    pub remote_clients: u32,
}

/// `connect 127.0.0.1:port` / `localhost` while listen host is up → in-process channel, not UDP client.
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
