//! Server → client user messages: wire = `u8 msg_id` + payload (name only at `um_register`).

use super::protocol::SVC;

/// `um_send` dest: all remote clients (+ local apply for non-engine messages).
pub const UM_DEST_BROADCAST: i32 = 0;

#[derive(Clone, Debug)]
pub struct UserMessageRpc {
    pub msg_id: u16,
    pub data: Vec<u8>,
}

impl UserMessageRpc {
    pub fn new(msg_id: u16, data: Vec<u8>) -> Self {
        Self { msg_id, data }
    }

    /// Wire body after `SVC::UserMessage`: `u8 msg_id` + payload (packet bounds = length).
    pub fn encode(&self) -> Vec<u8> {
        assert!(
            self.msg_id <= u8::MAX as u16,
            "user message id {} exceeds wire u8",
            self.msg_id
        );
        let mut out = Vec::with_capacity(1 + self.data.len());
        out.push(self.msg_id as u8);
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let Some((&id, data)) = payload.split_first() else {
            return None;
        };
        Some(Self {
            msg_id: id as u16,
            data: data.to_vec(),
        })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(1 + self.data.len());
        p.push(SVC::UserMessage as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

pub fn encode_set_view(net_id: u16) -> Vec<u8> {
    net_id.to_le_bytes().to_vec()
}

pub fn decode_set_view(data: &[u8]) -> Option<u16> {
    if data.len() < 2 {
        return None;
    }
    Some(u16::from_le_bytes([data[0], data[1]]))
}

pub fn encode_light_style(style: u8, pattern: &str) -> Vec<u8> {
    let bytes = pattern.as_bytes();
    let n = bytes.len().min(255);
    let mut out = Vec::with_capacity(2 + n);
    out.push(style);
    out.push(n as u8);
    out.extend_from_slice(&bytes[..n]);
    out
}

pub fn encode_map_name(name: &str) -> Vec<u8> {
    let bytes = name.as_bytes();
    let n = bytes.len().min(u16::MAX as usize);
    let mut out = Vec::with_capacity(2 + n);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..n]);
    out
}

pub fn decode_map_name(data: &[u8]) -> Option<String> {
    if data.len() < 2 {
        return None;
    }
    let n = u16::from_le_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + n {
        return None;
    }
    Some(String::from_utf8_lossy(&data[2..2 + n]).into_owned())
}

pub fn decode_light_style(data: &[u8]) -> Option<(u8, String)> {
    if data.len() < 2 {
        return None;
    }
    let style = data[0];
    let n = data[1] as usize;
    if data.len() < 2 + n {
        return None;
    }
    let pat = String::from_utf8_lossy(&data[2..2 + n]).into_owned();
    Some((style, pat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_view_roundtrip() {
        let rpc = UserMessageRpc::new(1, encode_set_view(42));
        let back = UserMessageRpc::decode(&rpc.encode()).unwrap();
        assert_eq!(back.msg_id, 1);
        assert_eq!(decode_set_view(&back.data), Some(42));
    }

    #[test]
    fn light_style_payload() {
        let data = encode_light_style(7, "mmamam");
        let (style, pat) = decode_light_style(&data).unwrap();
        assert_eq!(style, 7);
        assert_eq!(pat, "mmamam");
    }
}
