//! Join lifecycle shared by host (listen, dedicated, UDP client).

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClientSessionPhase {
    #[default]
    Idle,
    Init,
    Play,
}

#[derive(Default)]
pub struct SlotPlayGate {
    ready: HashSet<u32>,
}

impl SlotPlayGate {
    pub fn clear(&mut self) {
        self.ready.clear();
    }

    pub fn mark_ready(&mut self, slot: u32) {
        self.ready.insert(slot);
    }

    pub fn remove(&mut self, slot: u32) {
        self.ready.remove(&slot);
    }

    pub fn accepts_usercmd(&self, slot: u32) -> bool {
        self.ready.contains(&slot)
    }
}
