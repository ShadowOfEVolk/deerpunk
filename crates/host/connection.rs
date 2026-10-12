//! Client join lifecycle: **auth** → **init** → **play**.
//!
//! - **Auth:** OOB challenge + netcode `Secure` (UDP clients only).
//! - **Init:** per-slot SVC bundle (`PrefabDef` → snapshot → … → `InitDone`).
//! - **Play:** `UserCmd` + replication. Listen: [`Host::connect_to_server_inprocess`] (= `connect localhost`, transport [`crate::channel::Channel`]).

use std::collections::HashSet;

/// In-process client session (listen **or** remote UDP).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClientSessionPhase {
    #[default]
    Idle,
    /// Applying server init SVCs.
    Init,
    /// `InitDone` received — gameplay (`in_game`, UserCmd).
    Play,
}

/// Slots allowed to send `UserCmd` (UDP or channel).
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
