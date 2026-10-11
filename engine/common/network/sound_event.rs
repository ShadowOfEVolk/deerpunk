//! Sound event RPC — server → client ("play this sound").
//!
//! Wire format (little-endian, SVC::SoundEvent payload; future network boundary —
//! keep binary-simple):
//!
//! ```text
//! u8      kind           (0 = Play2D, 1 = Play3D, 2 = StartAttached, 3 = StopAttached)
//! u64     key            attached entity key (0 for one-shot kinds)
//! f32 x3  position       world units (Play3D / StartAttached)
//! u8      volume         0..=255 → linear 0..1
//! u8      name_len       event path length (bytes)
//! [u8]    event          ascii/utf8 path ("event:/…")
//! u8      param_count
//! per param:
//!   u8    param_len
//!   [u8]  param_name
//!   f32   param_value
//! ```
//!
//! `StopAttached` reads only `kind` + `key`; everything else may be zero.

pub const KIND_PLAY_2D: u8 = 0;
pub const KIND_PLAY_3D: u8 = 1;
pub const KIND_START_ATTACHED: u8 = 2;
pub const KIND_STOP_ATTACHED: u8 = 3;

#[derive(Clone, Debug, Default)]
pub struct SoundEventRpc {
    pub kind: u8,
    /// Attached entity key (0 for one-shot kinds).
    pub key: u64,
    pub position: [f32; 3],
    /// Linear 0..1 (clamped).
    pub volume: f32,
    pub event: String,
    pub params: Vec<(String, f32)>,
}

impl SoundEventRpc {
    pub fn play_2d(event: &str, volume: f32, params: Vec<(String, f32)>) -> Self {
        Self {
            kind: KIND_PLAY_2D,
            event: event.to_string(),
            volume,
            params,
            ..Default::default()
        }
    }

    pub fn play_3d(event: &str, position: [f32; 3], volume: f32, params: Vec<(String, f32)>) -> Self {
        Self {
            kind: KIND_PLAY_3D,
            event: event.to_string(),
            position,
            volume,
            params,
            ..Default::default()
        }
    }

    pub fn start_attached(key: u64, event: &str, position: [f32; 3], volume: f32) -> Self {
        Self {
            kind: KIND_START_ATTACHED,
            key,
            event: event.to_string(),
            position,
            volume,
            ..Default::default()
        }
    }

    pub fn stop_attached(key: u64) -> Self {
        Self {
            kind: KIND_STOP_ATTACHED,
            key,
            ..Default::default()
        }
    }

    /// Encode payload (without the SVC command byte).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48);
        out.push(self.kind);
        out.extend_from_slice(&self.key.to_le_bytes());
        for v in self.position {
            out.extend_from_slice(&v.to_le_bytes());
        }
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

    /// Decode payload (without the SVC command byte). `None` on malformed input.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor { data: payload, pos: 0 };
        let kind = c.u8()?;
        let rpc = Self {
            kind,
            key: c.u64()?,
            position: [c.f32()?, c.f32()?, c.f32()?],
            volume: c.u8()? as f32 / 255.0,
            event: c.string()?,
            params: {
                let count = c.u8()? as usize;
                let mut params = Vec::with_capacity(count.min(8));
                for _ in 0..count {
                    let name = c.string()?;
                    let value = c.f32()?;
                    params.push((name, value));
                }
                params
            },
        };
        Some(rpc)
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

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
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
