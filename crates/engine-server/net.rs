//! Network types shared by server and client UDP stacks.

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

pub use crate::client_net::ClientNet;
pub use crate::server_net::{
    is_connect_to_local_listen, parse_addr, ServerNet, DEFAULT_PORT, PROTOCOL_ID,
};
