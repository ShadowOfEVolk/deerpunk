//! Client UDP socket (connect + renet). Game wire = `CLC` / `SVC` bytes.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, SystemTime};

use deerpunk_engine_common::network::protocol::CLC;
use renet::{ConnectionConfig, DefaultChannel, RenetClient};
use renet_netcode::ClientAuthentication;

use crate::connect_auth;
use crate::net::NetHudStats;
use crate::server_net::PROTOCOL_ID;
use crate::udp_demux::DemuxClientTransport;

struct ClientSession {
    client: RenetClient,
    transport: DemuxClientTransport,
}

/// Outbound connection to a remote server (not loopback).
#[derive(Default)]
pub struct ClientNet {
    session: Option<ClientSession>,
}

impl ClientNet {
    pub fn is_connected(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.client.is_connected())
    }

    /// Transport up (connecting or connected).
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub fn disconnect(&mut self) {
        if let Some(s) = &mut self.session {
            s.transport.disconnect();
        }
        self.session = None;
        log::info!("[net] client disconnected");
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
        self.session = Some(ClientSession { client, transport });
        log::info!("[net] secure handshake to {server_addr} protocol={PROTOCOL_ID:#x}");
        Ok(())
    }

    pub fn poll(&mut self, dt: Duration) -> Vec<Vec<u8>> {
        let mut inbound_s2c = Vec::new();
        let Some(s) = &mut self.session else {
            return inbound_s2c;
        };
        s.client.update(dt);
        if let Err(e) = s.transport.update(dt, &mut s.client) {
            log::warn!("[net] client transport.update: {e}");
        }
        if s.client.is_connected() {
            while let Some(msg) = s.client.receive_message(DefaultChannel::ReliableOrdered) {
                inbound_s2c.push(msg.to_vec());
            }
            while let Some(msg) = s.client.receive_message(DefaultChannel::Unreliable) {
                inbound_s2c.push(msg.to_vec());
            }
        }
        if let Err(e) = s.transport.send_packets(&mut s.client) {
            log::warn!("[net] client send_packets: {e}");
        }
        inbound_s2c
    }

    pub fn send_c2s(&mut self, packet: Vec<u8>) {
        let Some(s) = &mut self.session else {
            return;
        };
        if !s.client.is_connected() {
            return;
        }
        let cmd = packet.first().copied().unwrap_or(0);
        let ch = if cmd == CLC::UserCmd as u8 {
            DefaultChannel::Unreliable
        } else {
            DefaultChannel::ReliableOrdered
        };
        s.client.send_message(ch, packet);
    }

    pub fn hud_stats(&self) -> NetHudStats {
        let Some(s) = &self.session else {
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
        let connected = s.client.is_connected();
        let info = s.client.network_info();
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
