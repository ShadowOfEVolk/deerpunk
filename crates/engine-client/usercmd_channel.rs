//! Outbound UserCmd ring + server ack (Xash `outgoing_sequence` / `incoming_acknowledged`).

use deerpunk_engine_common::network::{UserCmd, USERCMD_RING_MASK};

#[derive(Clone, Copy, Debug)]
pub struct StoredUserCmd {
    pub cmd: UserCmd,
    pub sent_time: f32,
}

#[derive(Debug)]
pub struct UsercmdChannel {
    /// Next sequence number to assign on send.
    pub outgoing_sequence: u32,
    /// Last `UserCmd::sequence` the server said it processed.
    pub incoming_acknowledged: u32,
    commands: [StoredUserCmd; USERCMD_RING_MASK + 1],
}

impl Default for UsercmdChannel {
    fn default() -> Self {
        Self {
            outgoing_sequence: 1,
            incoming_acknowledged: 0,
            commands: [StoredUserCmd {
                cmd: UserCmd::default(),
                sent_time: 0.0,
            }; USERCMD_RING_MASK + 1],
        }
    }
}

impl UsercmdChannel {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn note_sent(&mut self, cmd: UserCmd, sent_time: f32) -> UserCmd {
        let seq = self.outgoing_sequence;
        let mut cmd = cmd;
        cmd.sequence = seq;
        let slot = (seq as usize) & USERCMD_RING_MASK;
        self.commands[slot] = StoredUserCmd { cmd, sent_time };
        self.outgoing_sequence = seq + 1;
        cmd
    }

    pub fn set_ack(&mut self, ack: u32) {
        if ack > self.incoming_acknowledged {
            self.incoming_acknowledged = ack;
        }
    }

    pub fn get(&self, sequence: u32) -> Option<&UserCmd> {
        self.stored(sequence).map(|s| &s.cmd)
    }

    pub fn sent_time(&self, sequence: u32) -> Option<f32> {
        self.stored(sequence).map(|s| s.sent_time)
    }

    fn stored(&self, sequence: u32) -> Option<&StoredUserCmd> {
        if sequence == 0 || sequence >= self.outgoing_sequence {
            return None;
        }
        let slot = (sequence as usize) & USERCMD_RING_MASK;
        let stored = &self.commands[slot];
        if stored.cmd.sequence != sequence {
            return None;
        }
        Some(stored)
    }

    /// Unacknowledged sequences in order: `(ack+1)..outgoing_sequence` (capped by ring).
    pub fn pending_sequences(&self) -> impl Iterator<Item = u32> + Clone {
        let first = self.incoming_acknowledged + 1;
        let last = self.outgoing_sequence;
        first..last
    }
}
