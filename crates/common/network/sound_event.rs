//! Server → client sound instances (`sound_id` assigned on server).

use super::protocol::SVC;

/// Non-positional one-shot.
pub const SND_FLAG_2D: u8 = 1 << 0;
/// Loop until `SoundStop` (attached key = `sound_id` on client).
pub const SND_FLAG_LOOP: u8 = 1 << 1;
/// `net_id` field is set — client follows replicated entity when possible.
pub const SND_FLAG_NET_ENTITY: u8 = 1 << 2;

#[derive(Clone, Debug)]
pub struct SoundPlayRpc {
    pub sound_id: u32,
    pub flags: u8,
    pub net_id: u16,
    pub origin: [f32; 3],
    pub channel: u8,
    pub volume: f32,
    pub event: String,
    pub params: Vec<(String, f32)>,
}

#[derive(Clone, Copy, Debug)]
pub struct SoundStopRpc {
    pub sound_id: u32,
}

impl SoundPlayRpc {
    pub fn play_2d(sound_id: u32, event: &str, volume: f32, params: Vec<(String, f32)>) -> Self {
        Self {
            sound_id,
            flags: SND_FLAG_2D,
            net_id: 0,
            origin: [0.0, 0.0, 0.0],
            channel: 0,
            volume,
            event: event.to_string(),
            params,
        }
    }

    pub fn play_3d(
        sound_id: u32,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: Vec<(String, f32)>,
    ) -> Self {
        Self {
            sound_id,
            flags: 0,
            net_id: 0,
            origin,
            channel: 0,
            volume,
            event: event.to_string(),
            params,
        }
    }

    pub fn play_loop(
        sound_id: u32,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        net_id: u16,
    ) -> Self {
        let mut flags = SND_FLAG_LOOP;
        if net_id != 0 {
            flags |= SND_FLAG_NET_ENTITY;
        }
        Self {
            sound_id,
            flags,
            net_id,
            origin,
            channel: 0,
            volume,
            event: event.to_string(),
            params: Vec::new(),
        }
    }

    pub fn is_2d(&self) -> bool {
        self.flags & SND_FLAG_2D != 0
    }

    pub fn is_loop(&self) -> bool {
        self.flags & SND_FLAG_LOOP != 0
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 + self.event.len());
        out.extend_from_slice(&self.sound_id.to_le_bytes());
        out.push(self.flags);
        out.extend_from_slice(&self.net_id.to_le_bytes());
        for v in self.origin {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(self.channel);
        out.push((self.volume.clamp(0.0, 1.0) * 255.0).round() as u8);
        let event = self.event.as_bytes();
        out.push(event.len().min(255) as u8);
        out.extend_from_slice(&event[..event.len().min(255)]);
        out.push(self.params.len().min(8) as u8);
        for (name, value) in self.params.iter().take(8) {
            let name = name.as_bytes();
            out.push(name.len().min(255) as u8);
            out.extend_from_slice(&name[..name.len().min(255)]);
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor { data: payload, pos: 0 };
        Some(Self {
            sound_id: c.u32()?,
            flags: c.u8()?,
            net_id: c.u16()?,
            origin: [c.f32()?, c.f32()?, c.f32()?],
            channel: c.u8()?,
            volume: c.u8()? as f32 / 255.0,
            event: c.string()?,
            params: {
                let count = c.u8()? as usize;
                let mut params = Vec::with_capacity(count.min(8));
                for _ in 0..count {
                    params.push((c.string()?, c.f32()?));
                }
                params
            },
        })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(1 + self.event.len() + 32);
        p.push(SVC::SoundPlay as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

impl SoundStopRpc {
    pub fn new(sound_id: u32) -> Self {
        Self { sound_id }
    }

    pub fn encode(&self) -> Vec<u8> {
        self.sound_id.to_le_bytes().to_vec()
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        if payload.len() < 4 {
            return None;
        }
        Some(Self {
            sound_id: u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]),
        })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(5);
        p.push(SVC::SoundStop as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn u8(&mut self) -> Option<u8> {
        self.byte()
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(slice)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.byte()? as usize;
        let bytes = self.take(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}
