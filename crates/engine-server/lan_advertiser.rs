//! UDP LAN broadcast while hosting.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use deerpunk_engine_common::cvar_def::ARCHIVE;
use deerpunk_engine_common::network::lan::{
    self, encode_info, DEFAULT_GAME_PORT, LanAdvertiseState,
};

deerpunk_engine_common::cvar_def!(
    "sv_port",
    "28015",
    "UDP listen port for multiplayer (renet).",
    ARCHIVE
);
deerpunk_engine_common::cvar_def!(
    "hostname",
    "DarkDemo",
    "Server name shown in LAN browser.",
    ARCHIVE
);

const ADVERTISE_INTERVAL: Duration = Duration::from_millis(1500);

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
            port: if port == 0 {
                DEFAULT_GAME_PORT
            } else {
                port
            },
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
