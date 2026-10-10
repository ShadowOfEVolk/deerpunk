//! In-process client ↔ server channel (listen-server).
//!
//! UI / console send [`ClientCmd`]; host drains and routes to the server side.
//! Server→client game state is applied by host replication into the client world.

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
}

impl Channel {
    pub fn new() -> Self {
        Self {
            c2s: Mutex::new(VecDeque::new()),
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
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}
