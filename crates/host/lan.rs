//! LAN discovery — GoldSrc-style connectionless packets on `sv_port`.
//!
//! ```text
//! query:  ff ff ff ff "deerpunk_lan_query"
//! info:   ff ff ff ff "deerpunk_lan_info" \0 hostname \0 map \0 u16_le port \0 u8 players \0 u8 max
//! ```
//!
//! Host advertises via UDP broadcast (renet keeps the game socket). Browser binds the
//! same port with `SO_REUSEADDR` so announces are visible without demuxing renet.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

use crate::net::DEFAULT_PORT;

pub const HEADER: &[u8] = &[0xff, 0xff, 0xff, 0xff];
pub const QUERY_TAG: &[u8] = b"deerpunk_lan_query";
pub const INFO_TAG: &[u8] = b"deerpunk_lan_info";

const ADVERTISE_INTERVAL: Duration = Duration::from_millis(1500);
const SERVER_TTL: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
pub struct LanServerInfo {
    pub addr: SocketAddr,
    pub hostname: String,
    pub map: String,
    pub port: u16,
    pub players: u8,
    pub max_players: u8,
}

#[derive(Clone, Debug)]
struct SeenServer {
    info: LanServerInfo,
    last_seen: Instant,
}

/// Payload for outbound announces (filled by Host each tick).
#[derive(Clone, Debug, Default)]
pub struct LanAdvertiseState {
    pub hostname: String,
    pub map: String,
    pub port: u16,
    pub players: u8,
    pub max_players: u8,
}

pub struct LanAdvertiser {
    socket: UdpSocket,
    port: u16,
    last_send: Instant,
}

impl LanAdvertiser {
    pub fn new(port: u16) -> Result<Self, String> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("lan advertiser bind: {e}"))?;
        socket
            .set_broadcast(true)
            .map_err(|e| format!("lan advertiser broadcast: {e}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| format!("lan advertiser nonblocking: {e}"))?;
        Ok(Self {
            socket,
            port: if port == 0 { DEFAULT_PORT } else { port },
            last_send: Instant::now()
                .checked_sub(ADVERTISE_INTERVAL)
                .unwrap_or_else(Instant::now),
        })
    }

    pub fn tick(&mut self, state: &LanAdvertiseState) {
        if self.last_send.elapsed() < ADVERTISE_INTERVAL {
            return;
        }
        self.last_send = Instant::now();
        let pkt = encode_info(state);
        let bcast: SocketAddr = (Ipv4Addr::BROADCAST, self.port).into();
        let local: SocketAddr = (Ipv4Addr::LOCALHOST, self.port).into();
        let _ = self.socket.send_to(&pkt, bcast);
        let _ = self.socket.send_to(&pkt, local);
    }
}

pub struct LanBrowser {
    socket: Option<UdpSocket>,
    port: u16,
    seen: HashMap<String, SeenServer>,
}

impl Default for LanBrowser {
    fn default() -> Self {
        Self {
            socket: None,
            port: DEFAULT_PORT,
            seen: HashMap::new(),
        }
    }
}

impl LanBrowser {
    pub fn ensure_listening(&mut self, port: u16) -> Result<(), String> {
        let port = if port == 0 { DEFAULT_PORT } else { port };
        if self.socket.is_some() && self.port == port {
            return Ok(());
        }
        self.socket = None;
        self.port = port;
        let sock = bind_reuse_broadcast(port)?;
        self.socket = Some(sock);
        // Kick a query so hosts that demux (or future) can reply immediately.
        self.send_query();
        log::info!("[lan] browser listening on 0.0.0.0:{port}");
        Ok(())
    }

    pub fn stop(&mut self) {
        self.socket = None;
        self.seen.clear();
    }

    pub fn send_query(&self) {
        let Some(sock) = self.socket.as_ref() else {
            return;
        };
        let mut pkt = Vec::with_capacity(4 + QUERY_TAG.len());
        pkt.extend_from_slice(HEADER);
        pkt.extend_from_slice(QUERY_TAG);
        let bcast: SocketAddr = (Ipv4Addr::BROADCAST, self.port).into();
        let local: SocketAddr = (Ipv4Addr::LOCALHOST, self.port).into();
        let _ = sock.send_to(&pkt, bcast);
        let _ = sock.send_to(&pkt, local);
    }

    pub fn poll(&mut self) {
        let Some(sock) = self.socket.as_ref() else {
            return;
        };
        let mut buf = [0u8; 512];
        loop {
            match sock.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if let Some(mut info) = decode_info(&buf[..n]) {
                        // Prefer packet port; fall back to source port.
                        if info.port == 0 {
                            info.port = from.port();
                        }
                        info.addr = SocketAddr::new(from.ip(), info.port);
                        let key = info.addr.to_string();
                        self.seen.insert(
                            key,
                            SeenServer {
                                info,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            }
        }
        let now = Instant::now();
        self.seen
            .retain(|_, s| now.duration_since(s.last_seen) < SERVER_TTL);
    }

    pub fn servers(&self) -> Vec<LanServerInfo> {
        let mut v: Vec<_> = self.seen.values().map(|s| s.info.clone()).collect();
        v.sort_by(|a, b| a.addr.to_string().cmp(&b.addr.to_string()));
        v
    }
}

/// JSON list for Lua `eng_get_files_list("lan://servers")`.
static LAN_SERVERS_JSON: Mutex<String> = Mutex::new(String::new());

pub fn set_servers_json(servers: &[LanServerInfo]) {
    let mut arr = Vec::with_capacity(servers.len());
    for s in servers {
        arr.push(serde_json::json!({
            "addr": format!("{}:{}", s.addr.ip(), s.port),
            "ip": s.addr.ip().to_string(),
            "port": s.port,
            "hostname": s.hostname,
            "map": s.map,
            "players": s.players,
            "maxplayers": s.max_players,
        }));
    }
    let json = serde_json::Value::Array(arr).to_string();
    if let Ok(mut g) = LAN_SERVERS_JSON.lock() {
        *g = json;
    }
}

pub fn servers_json() -> String {
    LAN_SERVERS_JSON
        .lock()
        .ok()
        .map(|g| g.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "[]".into())
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
