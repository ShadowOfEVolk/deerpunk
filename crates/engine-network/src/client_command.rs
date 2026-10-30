//! Client → server console-style command string (Xash `clc_stringcmd`-style).

use crate::protocol::CLC;

/// Max UTF-8 bytes in one CLC command (excluding length prefix).
pub const MAX_CLIENT_COMMAND_LEN: usize = 256;

/// Wire after CLC discriminant: `u16` LE length + UTF-8 bytes.
pub fn decode_payload(payload: &[u8]) -> Option<&str> {
    if payload.len() < 2 {
        return None;
    }
    let len = u16::from_le_bytes(payload[0..2].try_into().ok()?) as usize;
    if len > MAX_CLIENT_COMMAND_LEN || payload.len() < 2 + len {
        return None;
    }
    std::str::from_utf8(&payload[2..2 + len]).ok()
}

/// Full CLC packet: `[CLC::ClientCommand][u16 len][utf8…]`.
pub fn encode_clc_packet(line: &str) -> Option<Vec<u8>> {
    let bytes = line.as_bytes();
    if bytes.len() > MAX_CLIENT_COMMAND_LEN {
        return None;
    }
    let mut pkt = Vec::with_capacity(1 + 2 + bytes.len());
    pkt.push(CLC::ClientCommand as u8);
    pkt.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    pkt.extend_from_slice(bytes);
    Some(pkt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let line = "selectslot 3";
        let pkt = encode_clc_packet(line).unwrap();
        assert_eq!(pkt[0], CLC::ClientCommand as u8);
        assert_eq!(decode_payload(&pkt[1..]).unwrap(), line);
    }
}
