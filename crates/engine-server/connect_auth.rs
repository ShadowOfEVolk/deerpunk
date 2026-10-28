//! Connectionless connect challenge → netcode `ConnectToken` (Secure handshake).
//!
//! ```text
//! challenge: ff ff ff ff "deerpunk_connect_ch" u64 client_id u64 protocol_id
//!            u8 addr_len + sockaddr bytes (v4: 4+2, v6: 16+2)
//! token:     ff ff ff ff "deerpunk_connect_tok" + ConnectToken::write()
//! ```

use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::{Duration, SystemTime};

use renet_netcode::generate_random_bytes;
use renet_netcode::{ConnectToken, NETCODE_KEY_BYTES};

use deerpunk_engine_common::network::lan::HEADER;

use crate::net::PROTOCOL_ID;

pub const CHALLENGE_TAG: &[u8] = b"deerpunk_connect_ch";
pub const TOKEN_TAG: &[u8] = b"deerpunk_connect_tok";

const TOKEN_EXPIRE_SECS: u64 = 300;
const CONNECT_TIMEOUT_SECS: i32 = 15;
const FETCH_TIMEOUT: Duration = Duration::from_secs(3);

pub fn key_path(gamedir: &Path) -> std::path::PathBuf {
    gamedir.join("netcode.key")
}

/// Load or create 32-byte netcode private key in gamedir.
pub fn load_or_create_private_key(gamedir: &Path) -> Result<[u8; NETCODE_KEY_BYTES], String> {
    let path = key_path(gamedir);
    if path.is_file() {
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if data.len() != NETCODE_KEY_BYTES {
            return Err(format!(
                "netcode.key must be {} bytes, got {}",
                NETCODE_KEY_BYTES,
                data.len()
            ));
        }
        let mut key = [0u8; NETCODE_KEY_BYTES];
        key.copy_from_slice(&data);
        return Ok(key);
    }
    let key = generate_random_bytes();
    std::fs::write(&path, key).map_err(|e| format!("write {}: {e}", path.display()))?;
    log::info!("[connect_auth] created {}", path.display());
    Ok(key)
}

/// Addresses clients may use in tokens (host-list check in netcode).
pub fn listen_public_addresses(bind: SocketAddr) -> Vec<SocketAddr> {
    let port = bind.port();
    let mut out = Vec::new();
    out.push(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    if !bind.ip().is_unspecified() && !bind.ip().is_loopback() {
        out.push(bind);
    }
    out
}

pub fn encode_challenge(client_id: u64, connect_addr: SocketAddr) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(HEADER);
    out.extend_from_slice(CHALLENGE_TAG);
    out.extend_from_slice(&client_id.to_le_bytes());
    out.extend_from_slice(&PROTOCOL_ID.to_le_bytes());
    encode_sockaddr(&mut out, connect_addr);
    out
}

fn encode_sockaddr(out: &mut Vec<u8>, addr: SocketAddr) {
    match addr {
        SocketAddr::V4(v4) => {
            out.push(4);
            out.extend_from_slice(&v4.ip().octets());
            out.extend_from_slice(&v4.port().to_le_bytes());
        }
        SocketAddr::V6(v6) => {
            out.push(6);
            out.extend_from_slice(&v6.ip().octets());
            out.extend_from_slice(&v6.port().to_le_bytes());
        }
    }
}

fn decode_sockaddr(buf: &[u8], i: usize) -> Option<(SocketAddr, usize)> {
    let fam = *buf.get(i)?;
    match fam {
        4 => {
            if i + 1 + 4 + 2 > buf.len() {
                return None;
            }
            let ip = Ipv4Addr::new(buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4]);
            let port = u16::from_le_bytes([buf[i + 5], buf[i + 6]]);
            Some((SocketAddr::from((ip, port)), i + 7))
        }
        6 => {
            if i + 1 + 16 + 2 > buf.len() {
                return None;
            }
            let mut oct = [0u8; 16];
            oct.copy_from_slice(&buf[i + 1..i + 17]);
            let port = u16::from_le_bytes([buf[i + 17], buf[i + 18]]);
            Some((SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::from(oct)), port), i + 19))
        }
        _ => None,
    }
}

pub fn decode_challenge(packet: &[u8]) -> Option<(u64, u64, SocketAddr)> {
    let min = HEADER.len() + CHALLENGE_TAG.len() + 16 + 7;
    if packet.len() < min {
        return None;
    }
    if &packet[..4] != HEADER || !packet[4..].starts_with(CHALLENGE_TAG) {
        return None;
    }
    let mut i = 4 + CHALLENGE_TAG.len();
    let client_id = u64::from_le_bytes(packet[i..i + 8].try_into().ok()?);
    i += 8;
    let protocol_id = u64::from_le_bytes(packet[i..i + 8].try_into().ok()?);
    i += 8;
    if protocol_id != PROTOCOL_ID {
        return None;
    }
    let (connect_addr, _) = decode_sockaddr(packet, i)?;
    Some((client_id, protocol_id, connect_addr))
}

pub fn encode_token_response(token: &ConnectToken) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    token
        .write(&mut body)
        .map_err(|e| format!("token write: {e}"))?;
    let mut out = Vec::with_capacity(4 + TOKEN_TAG.len() + body.len());
    out.extend_from_slice(HEADER);
    out.extend_from_slice(TOKEN_TAG);
    out.extend(body);
    Ok(out)
}

pub fn decode_token_response(packet: &[u8]) -> Option<ConnectToken> {
    let header_len = HEADER.len() + TOKEN_TAG.len();
    if packet.len() <= header_len {
        return None;
    }
    if &packet[..4] != HEADER || !packet[4..].starts_with(TOKEN_TAG) {
        return None;
    }
    let mut cur = Cursor::new(&packet[header_len..]);
    ConnectToken::read(&mut cur).ok()
}

/// Host: answer one challenge if packet matches.
/// `Some(addr)` → host should add `addr` to netcode public list and re-listen.
pub fn try_answer_challenge(
    packet: &[u8],
    reply_to: SocketAddr,
    private_key: &[u8; NETCODE_KEY_BYTES],
    public_addresses: &[SocketAddr],
    listen_port: u16,
    socket: &UdpSocket,
) -> Option<Option<SocketAddr>> {
    let Some((client_id, _proto, connect_addr)) = decode_challenge(packet) else {
        return None;
    };
    if connect_addr.port() != listen_port {
        log::debug!("[connect_auth] reject challenge: port mismatch");
        return Some(None);
    }
    if !public_addresses.contains(&connect_addr) {
        log::info!(
            "[connect_auth] need public addr {:?} (from {})",
            connect_addr,
            reply_to
        );
        return Some(Some(connect_addr));
    }
    // No password / extra auth yet — always approve.
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let token = match ConnectToken::generate(
        now,
        PROTOCOL_ID,
        TOKEN_EXPIRE_SECS,
        client_id,
        CONNECT_TIMEOUT_SECS,
        vec![connect_addr],
        None,
        private_key,
    ) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[connect_auth] token generate: {:?}", e);
            return Some(None);
        }
    };
    match encode_token_response(&token) {
        Ok(pkt) => {
            let _ = socket.send_to(&pkt, reply_to);
            log::info!(
                "[connect_auth] token → {} client_id={client_id} connect={connect_addr}",
                reply_to
            );
        }
        Err(e) => log::warn!("[connect_auth] encode: {e}"),
    }
    Some(None)
}

/// Client: connectionless challenge → token on an existing UDP socket (same socket as netcode).
pub fn fetch_connect_token_on_socket(
    sock: &UdpSocket,
    server_addr: SocketAddr,
    client_id: u64,
) -> Result<ConnectToken, String> {
    let challenge = encode_challenge(client_id, server_addr);
    let deadline = std::time::Instant::now() + FETCH_TIMEOUT;
    let mut buf = [0u8; 2048];
    let mut last_send = std::time::Instant::now() - Duration::from_secs(1);
    loop {
        if std::time::Instant::now() >= deadline {
            return Err("connect challenge timeout (no token)".into());
        }
        if last_send.elapsed() > Duration::from_millis(400) {
            let challenge = encode_challenge(client_id, server_addr);
            sock.send_to(&challenge, server_addr)
                .map_err(|e| format!("send challenge: {e}"))?;
            last_send = std::time::Instant::now();
        }
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                if from.ip() != server_addr.ip() && !server_addr.ip().is_unspecified() {
                    continue;
                }
                if let Some(tok) = decode_token_response(&buf[..n]) {
                    if tok.client_id != client_id {
                        continue;
                    }
                    return Ok(tok);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
            Err(e) => return Err(format!("recv: {e}")),
        }
    }
}

/// Ephemeral socket (tests / tools).
pub fn fetch_connect_token(
    server_addr: SocketAddr,
    client_id: u64,
) -> Result<ConnectToken, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind: {e}"))?;
    sock.set_nonblocking(true).map_err(|e| format!("nonblocking: {e}"))?;
    fetch_connect_token_on_socket(&sock, server_addr, client_id)
}
