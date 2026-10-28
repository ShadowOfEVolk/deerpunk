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
//! u32  sequence  // client cmd number (offset 36, GoldSrc-style ack)
//! f32  claimed_x,y,z  // offset 40 when `USERCMD_FLAG_CLAIMED_FEET`
//! u8   pad[8]
//! ```

pub const USERCMD_VERSION: u16 = 1;
pub const USERCMD_SIZE: usize = 64;
/// Ring size for outbound cmd history (`sequence & (N-1)`).
pub const USERCMD_RING_SIZE: usize = 64;
pub const USERCMD_RING_MASK: usize = USERCMD_RING_SIZE - 1;

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
/// Secondary attack (typically mouse2): place block in voxel games.
pub const BUTTON_ATTACK2: u64 = 1 << 9;
/// Tertiary attack (typically mouse3): pick block / tool action.
pub const BUTTON_ATTACK3: u64 = 1 << 10;
pub const BUTTON_DROP: u64 = 1 << 11;

/// Client sends feet with the cmd (`sv_movement_authority` validate mode).
pub const USERCMD_FLAG_CLAIMED_FEET: u16 = 1 << 0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserCmd {
    pub version: u16,
    pub flags: u16,
    pub yaw: f32,
    pub pitch: f32,
    pub buttons: u64,
    pub axes: [f32; AXIS_COUNT],
    /// Monotonic per client; server echoes ack via [`super::move_ack::UserCmdAckRpc`].
    pub sequence: u32,
    pub claimed_feet: [f32; 3],
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
            sequence: 0,
            claimed_feet: [0.0; 3],
        }
    }
}

impl UserCmd {
    pub fn set_claimed_feet(&mut self, feet: [f32; 3]) {
        self.flags |= USERCMD_FLAG_CLAIMED_FEET;
        self.claimed_feet = feet;
    }

    pub fn clear_claimed_feet(&mut self) {
        self.flags &= !USERCMD_FLAG_CLAIMED_FEET;
    }

    pub fn claimed_feet(&self) -> Option<[f32; 3]> {
        if self.flags & USERCMD_FLAG_CLAIMED_FEET != 0 {
            Some(self.claimed_feet)
        } else {
            None
        }
    }

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
        out[36..40].copy_from_slice(&self.sequence.to_le_bytes());
        if self.flags & USERCMD_FLAG_CLAIMED_FEET != 0 {
            for i in 0..3 {
                let o = 40 + i * 4;
                out[o..o + 4].copy_from_slice(&self.claimed_feet[i].to_le_bytes());
            }
        }
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
        let sequence = u32::from_le_bytes(bytes[36..40].try_into().ok()?);
        let mut claimed_feet = [0.0f32; 3];
        if flags & USERCMD_FLAG_CLAIMED_FEET != 0 {
            for i in 0..3 {
                let o = 40 + i * 4;
                claimed_feet[i] = f32::from_le_bytes(bytes[o..o + 4].try_into().ok()?);
            }
        }
        Some(Self {
            version,
            flags,
            yaw,
            pitch,
            buttons,
            axes,
            sequence,
            claimed_feet,
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
