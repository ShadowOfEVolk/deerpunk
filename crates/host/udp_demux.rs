//! Single UDP socket: GoldSrc-style OOB demux → netcode/renet.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use renet::{ClientId, RenetClient, RenetServer};
use renet_netcode::{ClientAuthentication, NetcodeTransportError, ServerConfig};
use renetcode::{
    NetcodeClient, NetcodeServer, ServerResult, NETCODE_MAX_PACKET_BYTES,
};

use crate::connect_auth;
use crate::lan::{self, LanAdvertiseState};

fn handle_server_result(
    server_result: ServerResult,
    socket: &UdpSocket,
    reliable_server: &mut RenetServer,
) {
    let send_packet = |packet: &[u8], addr: SocketAddr| {
        if let Err(err) = socket.send_to(packet, addr) {
            log::error!("[net] send to {addr}: {err}");
        }
    };

    match server_result {
        ServerResult::None => {}
        ServerResult::PacketToSend { payload, addr } => send_packet(payload, addr),
        ServerResult::Payload { client_id, payload } => {
            if let Err(e) = reliable_server.process_packet_from(payload, client_id) {
                log::error!("[net] renet payload client {client_id}: {e}");
            }
        }
        ServerResult::ClientConnected {
            client_id,
            user_data: _,
            addr,
            payload,
        } => {
            reliable_server.add_connection(client_id);
            send_packet(payload, addr);
        }
        ServerResult::ClientDisconnected {
            client_id,
            addr,
            payload,
        } => {
            reliable_server.remove_connection(client_id);
            if let Some(payload) = payload {
                send_packet(payload, addr);
            }
        }
    }
}

pub struct DemuxServerTransport {
    socket: UdpSocket,
    netcode_server: NetcodeServer,
    buffer: [u8; NETCODE_MAX_PACKET_BYTES],
    listen_port: u16,
}

impl DemuxServerTransport {
    pub fn new(
        server_config: ServerConfig,
        socket: UdpSocket,
        listen_port: u16,
    ) -> Result<Self, io::Error> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            netcode_server: NetcodeServer::new(server_config),
            buffer: [0; NETCODE_MAX_PACKET_BYTES],
            listen_port,
        })
    }

    pub fn addresses(&self) -> Vec<SocketAddr> {
        self.netcode_server.addresses()
    }

    pub fn client_addr(&self, client_id: ClientId) -> Option<SocketAddr> {
        self.netcode_server.client_addr(client_id)
    }

    pub fn disconnect_all(&mut self, server: &mut RenetServer) {
        for client_id in self.netcode_server.clients_id() {
            let server_result = self.netcode_server.disconnect(client_id);
            handle_server_result(server_result, &self.socket, server);
        }
    }

    pub fn send_packets(&mut self, server: &mut RenetServer) {
        'clients: for client_id in server.clients_id() {
            let packets = server.get_packets_to_send(client_id).unwrap();
            for packet in packets {
                match self.netcode_server.generate_payload_packet(client_id, &packet) {
                    Ok((addr, payload)) => {
                        if let Err(e) = self.socket.send_to(payload, addr) {
                            log::error!("[net] send payload to {client_id} ({addr}): {e}");
                            continue 'clients;
                        }
                    }
                    Err(e) => {
                        log::error!("[net] encrypt payload for {client_id}: {e}");
                        continue 'clients;
                    }
                }
            }
        }
    }

    /// `Some(addr)` → add to netcode public list and re-listen.
    pub fn update(
        &mut self,
        duration: Duration,
        server: &mut RenetServer,
        private_key: &[u8; 32],
        public_addresses: &[SocketAddr],
        lan_reply: Option<&LanAdvertiseState>,
    ) -> Result<Option<SocketAddr>, NetcodeTransportError> {
        self.netcode_server.update(duration);
        let mut need_public = None;

        loop {
            match self.socket.recv_from(&mut self.buffer) {
                Ok((len, addr)) => {
                    let packet = &mut self.buffer[..len];
                    if lan::has_connectionless_header(packet) {
                        if let Some(r) = connect_auth::try_answer_challenge(
                            packet,
                            addr,
                            private_key,
                            public_addresses,
                            self.listen_port,
                            &self.socket,
                        ) {
                            if let Some(pub_addr) = r {
                                need_public = Some(pub_addr);
                            }
                        } else if lan::is_connectionless_query(packet) {
                            if let Some(state) = lan_reply {
                                let reply = lan::encode_info(state);
                                let _ = self.socket.send_to(&reply, addr);
                            }
                        }
                        continue;
                    }
                    let result = self.netcode_server.process_packet(addr, packet);
                    handle_server_result(result, &self.socket, server);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => break,
                Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e.into()),
            }
        }

        for client_id in self.netcode_server.clients_id() {
            let server_result = self.netcode_server.update_client(client_id);
            handle_server_result(server_result, &self.socket, server);
        }

        for disconnection_id in server.disconnections_id() {
            let server_result = self.netcode_server.disconnect(disconnection_id);
            handle_server_result(server_result, &self.socket, server);
        }

        Ok(need_public)
    }
}

pub struct DemuxClientTransport {
    socket: UdpSocket,
    netcode_client: NetcodeClient,
    buffer: [u8; NETCODE_MAX_PACKET_BYTES],
}

impl DemuxClientTransport {
    pub fn new(
        current_time: Duration,
        authentication: ClientAuthentication,
        socket: UdpSocket,
    ) -> Result<Self, renetcode::NetcodeError> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            netcode_client: NetcodeClient::new(current_time, authentication)?,
            buffer: [0; NETCODE_MAX_PACKET_BYTES],
        })
    }

    pub fn client_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn disconnect(&mut self) {
        if self.netcode_client.is_disconnected() {
            return;
        }
        match self.netcode_client.disconnect() {
            Ok((addr, packet)) => {
                let _ = self.socket.send_to(packet, addr);
            }
            Err(e) => log::error!("[net] disconnect packet: {e}"),
        }
    }

    pub fn send_packets(&mut self, client: &mut RenetClient) -> Result<(), NetcodeTransportError> {
        if let Some(reason) = self.netcode_client.disconnect_reason() {
            return Err(renetcode::NetcodeError::Disconnected(reason).into());
        }
        let packets = client.get_packets_to_send();
        for packet in packets {
            let (addr, payload) = self.netcode_client.generate_payload_packet(&packet)?;
            self.socket.send_to(payload, addr)?;
        }
        Ok(())
    }

    pub fn update(
        &mut self,
        duration: Duration,
        client: &mut RenetClient,
    ) -> Result<(), NetcodeTransportError> {
        use renetcode::NetcodeError;

        if let Some(reason) = self.netcode_client.disconnect_reason() {
            client.disconnect_due_to_transport();
            return Err(NetcodeError::Disconnected(reason).into());
        }

        if let Some(error) = client.disconnect_reason() {
            let (addr, disconnect_packet) = self.netcode_client.disconnect()?;
            self.socket.send_to(disconnect_packet, addr)?;
            return Err(error.into());
        }

        if self.netcode_client.is_connected() {
            client.set_connected();
        } else if self.netcode_client.is_connecting() {
            client.set_connecting();
        }

        let server_addr = self.netcode_client.server_addr();
        loop {
            let packet = match self.socket.recv_from(&mut self.buffer) {
                Ok((len, addr)) => {
                    if lan::has_connectionless_header(&self.buffer[..len]) {
                        continue;
                    }
                    if addr != server_addr {
                        log::debug!("[net] discard packet from {addr}");
                        continue;
                    }
                    &mut self.buffer[..len]
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => break,
                Err(e) => return Err(NetcodeTransportError::IO(e)),
            };

            if let Some(payload) = self.netcode_client.process_packet(packet) {
                client.process_packet(payload);
            }
        }

        if let Some((packet, addr)) = self.netcode_client.update(duration) {
            self.socket.send_to(packet, addr)?;
        }

        Ok(())
    }
}
