//! GoldSrc-style connectionless LAN packets on the game UDP port.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use socket2::{Domain, Protocol, Socket, Type};

pub const DEFAULT_GAME_PORT: u16 = 28015;

pub const HEADER: &[u8] = &[0xff, 0xff, 0xff, 0xff];
pub const QUERY_TAG: &[u8] = b"deerpunk_lan_query";
pub const INFO_TAG: &[u8] = b"deerpunk_lan_info";

#[derive(Clone, Debug)]
pub struct LanServerInfo {
    pub addr: SocketAddr,
    pub hostname: String,
    pub map: String,
    pub port: u16,
    pub players: u8,
    pub max_players: u8,
}

#[derive(Clone, Debug, Default)]
pub struct LanAdvertiseState {
    pub hostname: String,
    pub map: String,
    pub port: u16,
    pub players: u8,
    pub max_players: u8,
}

pub fn encode_info(state: &LanAdvertiseState) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(HEADER);
    out.extend_from_slice(INFO_TAG);
    out.push(0);
    push_cstr(&mut out, &state.hostname);
    push_cstr(&mut out, &state.map);
    out.extend_from_slice(&state.port.to_le_bytes());
    out.push(state.players);
    out.push(state.max_players);
    out
}

pub fn decode_info(packet: &[u8]) -> Option<LanServerInfo> {
    if packet.len() < HEADER.len() + INFO_TAG.len() + 1 {
        return None;
    }
    if &packet[..4] != HEADER {
        return None;
    }
    let rest = &packet[4..];
    if !rest.starts_with(INFO_TAG) {
        return None;
    }
    let mut i = INFO_TAG.len();
    if rest.get(i) != Some(&0) {
        return None;
    }
    i += 1;
    let (hostname, i) = read_cstr(rest, i)?;
    let (map, i) = read_cstr(rest, i)?;
    if i + 4 > rest.len() {
        return None;
    }
    let port = u16::from_le_bytes([rest[i], rest[i + 1]]);
    let players = rest[i + 2];
    let max_players = rest[i + 3];
    Some(LanServerInfo {
        addr: ([0, 0, 0, 0], port).into(),
        hostname,
        map,
        port,
        players,
        max_players,
    })
}

pub fn has_connectionless_header(packet: &[u8]) -> bool {
    packet.len() >= HEADER.len() && &packet[..4] == HEADER
}

pub fn is_connectionless_query(packet: &[u8]) -> bool {
    has_connectionless_header(packet) && packet[4..].starts_with(QUERY_TAG)
}

fn push_cstr(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(63);
    out.extend_from_slice(&b[..n]);
    out.push(0);
}

fn read_cstr(buf: &[u8], start: usize) -> Option<(String, usize)> {
    let mut i = start;
    while i < buf.len() && buf[i] != 0 {
        i += 1;
    }
    if i >= buf.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&buf[start..i]).into_owned();
    Some((s, i + 1))
}

pub fn bind_reuse_broadcast(port: u16) -> Result<UdpSocket, String> {
    let addr: SocketAddr = (Ipv4Addr::UNSPECIFIED, port).into();
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| format!("lan socket: {e}"))?;
    socket
        .set_reuse_address(true)
        .map_err(|e| format!("lan reuseaddr: {e}"))?;
    #[cfg(unix)]
    {
        let _ = socket.set_reuse_port(true);
    }
    socket
        .set_nonblocking(true)
        .map_err(|e| format!("lan nonblocking: {e}"))?;
    socket
        .set_broadcast(true)
        .map_err(|e| format!("lan broadcast: {e}"))?;
    socket
        .bind(&addr.into())
        .map_err(|e| format!("lan bind {addr}: {e}"))?;
    Ok(socket.into())
}
