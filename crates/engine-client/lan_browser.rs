//! Menu LAN server browser (connectionless on `sv_port`).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use deerpunk_engine_common::network::lan::{
    bind_reuse_broadcast, decode_info, DEFAULT_GAME_PORT, HEADER, LanServerInfo, QUERY_TAG,
};

const SERVER_TTL: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
struct SeenServer {
    info: LanServerInfo,
    last_seen: Instant,
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
            port: DEFAULT_GAME_PORT,
            seen: HashMap::new(),
        }
    }
}

impl LanBrowser {
    pub fn ensure_listening(&mut self, port: u16) -> Result<(), String> {
        let port = if port == 0 { DEFAULT_GAME_PORT } else { port };
        if self.socket.is_some() && self.port == port {
            return Ok(());
        }
        self.socket = None;
        self.port = port;
        let sock = bind_reuse_broadcast(port)?;
        self.socket = Some(sock);
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
