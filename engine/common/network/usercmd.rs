//! Client → server UserCmd (fixed 64-byte blob).
//!
//! Layout (little-endian). Schema negotiation on connect is reserved for later;
//! v1 client/server share this layout via the common crate.
//!
//! ```text
//! u16  version = 1
//! u16  flags / reserved
//! f32  yaw
//! f32  pitch
//! u64  buttons
//! f32  axes[4]   // 0=forward, 1=side, 2=up, 3=reserved
//! u8   pad[24]   // → 64 bytes total
//! ```

pub const USERCMD_VERSION: u16 = 1;
pub const USERCMD_SIZE: usize = 64;

pub const AXIS_FORWARD: usize = 0;
pub const AXIS_SIDE: usize = 1;
pub const AXIS_UP: usize = 2;
pub const AXIS_RESERVED: usize = 3;
pub const AXIS_COUNT: usize = 4;

pub const BUTTON_ATTACK: u64 = 1 << 0;
pub const BUTTON_JUMP: u64 = 1 << 1;
pub const BUTTON_DUCK: u64 = 1 << 2;
pub const BUTTON_FORWARD: u64 = 1 << 3;
pub const BUTTON_BACK: u64 = 1 << 4;
pub const BUTTON_MOVELEFT: u64 = 1 << 5;
pub const BUTTON_MOVERIGHT: u64 = 1 << 6;
pub const BUTTON_USE: u64 = 1 << 7;
pub const BUTTON_SPEED: u64 = 1 << 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserCmd {
    pub version: u16,
    pub flags: u16,
    pub yaw: f32,
    pub pitch: f32,
    pub buttons: u64,
    pub axes: [f32; AXIS_COUNT],
}

impl Default for UserCmd {
    fn default() -> Self {
        Self {
            version: USERCMD_VERSION,
            flags: 0,
            yaw: 0.0,
            pitch: 0.0,
            buttons: 0,
            axes: [0.0; AXIS_COUNT],
        }
    }
}

impl UserCmd {
    pub fn encode(&self) -> [u8; USERCMD_SIZE] {
        let mut out = [0u8; USERCMD_SIZE];
        out[0..2].copy_from_slice(&self.version.to_le_bytes());
        out[2..4].copy_from_slice(&self.flags.to_le_bytes());
        out[4..8].copy_from_slice(&self.yaw.to_le_bytes());
        out[8..12].copy_from_slice(&self.pitch.to_le_bytes());
        out[12..20].copy_from_slice(&self.buttons.to_le_bytes());
        for i in 0..AXIS_COUNT {
            let o = 20 + i * 4;
            out[o..o + 4].copy_from_slice(&self.axes[i].to_le_bytes());
        }
        // pad[24] already zero
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < USERCMD_SIZE {
            return None;
        }
        let version = u16::from_le_bytes(bytes[0..2].try_into().ok()?);
        if version != USERCMD_VERSION {
            return None;
        }
        let flags = u16::from_le_bytes(bytes[2..4].try_into().ok()?);
        let yaw = f32::from_le_bytes(bytes[4..8].try_into().ok()?);
        let pitch = f32::from_le_bytes(bytes[8..12].try_into().ok()?);
        let buttons = u64::from_le_bytes(bytes[12..20].try_into().ok()?);
        let mut axes = [0.0f32; AXIS_COUNT];
        for i in 0..AXIS_COUNT {
            let o = 20 + i * 4;
            axes[i] = f32::from_le_bytes(bytes[o..o + 4].try_into().ok()?);
        }
        Some(Self {
            version,
            flags,
            yaw,
            pitch,
            buttons,
            axes,
        })
    }

    /// CLC packet: `CLC::UserCmd` byte + 64-byte payload.
    pub fn encode_clc_packet(&self) -> [u8; 1 + USERCMD_SIZE] {
        let mut pkt = [0u8; 1 + USERCMD_SIZE];
        pkt[0] = super::protocol::CLC::UserCmd as u8;
        pkt[1..].copy_from_slice(&self.encode());
        pkt
    }

    pub fn decode_clc_packet(packet: &[u8]) -> Option<Self> {
        if packet.is_empty() {
            return None;
        }
        if packet[0] != super::protocol::CLC::UserCmd as u8 {
            return None;
        }
        Self::decode(&packet[1..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut cmd = UserCmd::default();
        cmd.yaw = 1.25;
        cmd.pitch = -0.5;
        cmd.buttons = BUTTON_USE | BUTTON_SPEED;
        cmd.axes[AXIS_FORWARD] = 1.0;
        cmd.axes[AXIS_SIDE] = -0.5;
        let bytes = cmd.encode();
        assert_eq!(bytes.len(), USERCMD_SIZE);
        let back = UserCmd::decode(&bytes).unwrap();
        assert_eq!(back, cmd);
        let pkt = cmd.encode_clc_packet();
        assert_eq!(UserCmd::decode_clc_packet(&pkt), Some(cmd));
    }
}
