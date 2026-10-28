//! Server → client UserCmd acknowledgment (GoldSrc `incoming_acknowledged`).

use super::protocol::SVC;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserCmdAckRpc {
    /// Last client `UserCmd::sequence` applied on the server this tick.
    pub ack_sequence: u32,
    /// Server simulation tick (monotonic, wraps).
    pub server_tick: u32,
}

impl UserCmdAckRpc {
    pub const PAYLOAD_LEN: usize = 8;

    pub fn encode_payload(&self) -> [u8; Self::PAYLOAD_LEN] {
        let mut out = [0u8; Self::PAYLOAD_LEN];
        out[0..4].copy_from_slice(&self.ack_sequence.to_le_bytes());
        out[4..8].copy_from_slice(&self.server_tick.to_le_bytes());
        out
    }

    pub fn decode_payload(payload: &[u8]) -> Option<Self> {
        if payload.len() < Self::PAYLOAD_LEN {
            return None;
        }
        let ack_sequence = u32::from_le_bytes(payload[0..4].try_into().ok()?);
        let server_tick = u32::from_le_bytes(payload[4..8].try_into().ok()?);
        Some(Self {
            ack_sequence,
            server_tick,
        })
    }

    pub fn encode_packet(&self) -> Vec<u8> {
        let mut pkt = Vec::with_capacity(1 + Self::PAYLOAD_LEN);
        pkt.push(SVC::UserCmdAck as u8);
        pkt.extend_from_slice(&self.encode_payload());
        pkt
    }

    pub fn decode_packet(packet: &[u8]) -> Option<Self> {
        let Some((&cmd, payload)) = packet.split_first() else {
            return None;
        };
        if cmd != SVC::UserCmdAck as u8 {
            return None;
        }
        Self::decode_payload(payload)
    }
}
