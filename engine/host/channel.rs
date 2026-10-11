//! In-process client ↔ server channel (listen-server).
//!
//! UI / console send [`ClientCmd`]; host drains and routes to the server side.
//! Server→client RPC packets (SVC payloads, binary) queue in `s2c` and are
//! applied by the host frame on the client — the same bytes will later travel
//! over a real network channel.

use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub enum ClientCmd {
    /// Start / change map (enables server world).
    Map(String),
    /// Tear down server world (disable).
    Disconnect,
}

pub struct Channel {
    c2s: Mutex<VecDeque<ClientCmd>>,
    /// Binary client→server packets (`CLC` cmd byte + payload), e.g. UserCmd.
    c2s_bytes: Mutex<VecDeque<Vec<u8>>>,
    /// Server→client RPC packets (SVC payload bytes; SVC command byte included).
    s2c: Mutex<VecDeque<Vec<u8>>>,
}

impl Channel {
    pub fn new() -> Self {
        Self {
            c2s: Mutex::new(VecDeque::new()),
            c2s_bytes: Mutex::new(VecDeque::new()),
            s2c: Mutex::new(VecDeque::new()),
        }
    }

    pub fn push_client(&self, cmd: ClientCmd) {
        if let Ok(mut q) = self.c2s.lock() {
            q.push_back(cmd);
        }
    }

    pub fn drain_client(&self) -> Vec<ClientCmd> {
        let Ok(mut q) = self.c2s.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }

    /// Queue a binary client→server packet (`CLC` cmd byte + payload).
    pub fn push_client_bytes(&self, packet: Vec<u8>) {
        if packet.is_empty() {
            return;
        }
        if let Ok(mut q) = self.c2s_bytes.lock() {
            q.push_back(packet);
        }
    }

    /// Drain binary client→server packets (oldest first).
    pub fn drain_client_bytes(&self) -> Vec<Vec<u8>> {
        let Ok(mut q) = self.c2s_bytes.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }

    /// Queue a binary server→client packet (`SVC` cmd byte + payload).
    pub fn push_server(&self, packet: Vec<u8>) {
        if let Ok(mut q) = self.s2c.lock() {
            q.push_back(packet);
        }
    }

    /// Drain queued server→client packets (oldest first).
    pub fn drain_server(&self) -> Vec<Vec<u8>> {
        let Ok(mut q) = self.s2c.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}
