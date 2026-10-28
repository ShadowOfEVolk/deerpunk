//! In-process **loopback** link (GoldSrc/Xash-style listen server).
//!
//! Carries the same **game wire** bytes as UDP (`CLC` / `SVC` payloads from
//! [`protocol`]). Does not use renet — that stack is only for remote clients.
//!
//! High-level map / disconnect are not on the wire; they use [`HostCommand`].

use std::collections::VecDeque;
use std::sync::Mutex;

/// Console / UI → server control (not `CLC` / `SVC`).
#[derive(Clone, Debug)]
pub enum HostCommand {
    /// `map` / `changelevel` — enable server world.
    LoadMap(String),
    /// `disconnect` on listen — tear down server world.
    StopServer,
    /// Server gamelib finished `server_activated` — main thread runs in-process connect.
    InprocessConnect,
}

/// Single-process client ↔ server packet queues (thread-safe for future tooling).
#[derive(Default)]
pub struct NetLoopback {
    host_to_sv: Mutex<VecDeque<HostCommand>>,
    cl_to_sv: Mutex<VecDeque<Vec<u8>>>,
    sv_to_cl: Mutex<VecDeque<Vec<u8>>>,
}

impl NetLoopback {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Host control plane ───────────────────────────────────────

    pub fn post_host_command(&self, cmd: HostCommand) {
        if let Ok(mut q) = self.host_to_sv.lock() {
            q.push_back(cmd);
        }
    }

    pub fn drain_host_commands(&self) -> Vec<HostCommand> {
        let Ok(mut q) = self.host_to_sv.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }

    // ── Game wire (CLC / SVC bytes) ──────────────────────────────

    /// Client → server (`CLC` packet: cmd byte + payload).
    pub fn send_client_packet(&self, packet: Vec<u8>) {
        if packet.is_empty() {
            return;
        }
        if let Ok(mut q) = self.cl_to_sv.lock() {
            q.push_back(packet);
        }
    }

    /// Server side: drain pending `CLC` (oldest first).
    pub fn receive_client_packets(&self) -> Vec<Vec<u8>> {
        let Ok(mut q) = self.cl_to_sv.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }

    /// Server → client (`SVC` packet).
    pub fn send_server_packet(&self, packet: Vec<u8>) {
        if packet.is_empty() {
            return;
        }
        if let Ok(mut q) = self.sv_to_cl.lock() {
            q.push_back(packet);
        }
    }

    /// Client side: drain pending `SVC` (oldest first).
    pub fn receive_server_packets(&self) -> Vec<Vec<u8>> {
        let Ok(mut q) = self.sv_to_cl.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }
}
