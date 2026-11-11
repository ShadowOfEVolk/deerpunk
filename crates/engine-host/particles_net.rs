//! `svc emit_particles` — server → client one-shot particle burst.
//!
//! Server gamelib calls [`Host::particles_emit`] (FFI `v1:particles:emit`);
//! the burst is broadcast to play-phase clients and dispatched on the client
//! post-net-apply path into [`ClientEngine::spawn_particles_burst`].
//!
//! Wire layout (after the SVC cmd byte):
//! `u8 flags` + effect string (`u8` len + UTF-8) + `pos[3] f32` + `vel[3] f32`
//! + optional `SpawnPayload` fields, each gated by a presence bit:
//! count `u32`, scale `f32`, speed `f32`, tint `4×u8` (0..1 → 0..255),
//! dir `3×f32`, sprite (`u8` kind: 0 = uv rect `4×f32`, 1 = frame `u32`),
//! texture (`u8` len + UTF-8).

use std::sync::Mutex;

use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_particles::{SpawnPayload, SpriteOverride};

use crate::cmd::CmdSystem;
use crate::cmd::CommandRegistration;
use crate::Host;

/// Presence bits for the optional `SpawnPayload` fields.
const FLAG_COUNT: u8 = 1 << 0;
const FLAG_SCALE: u8 = 1 << 1;
const FLAG_SPEED: u8 = 1 << 2;
const FLAG_TINT: u8 = 1 << 3;
const FLAG_DIR: u8 = 1 << 4;
const FLAG_SPRITE: u8 = 1 << 5;
/// When set with [`FLAG_SPRITE`], the override is a frame index (else a UV rect).
const FLAG_SPRITE_FRAME: u8 = 1 << 6;
const FLAG_TEXTURE: u8 = 1 << 7;

/// One server-emitted particle burst (mirror of the client spawn arguments).
#[derive(Clone, Debug, PartialEq)]
pub struct ParticlesEmitRpc {
    pub effect: String,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub payload: SpawnPayload,
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

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(slice)
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

fn push_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn encode_payload_fields(out: &mut Vec<u8>, flags: &mut u8, payload: &SpawnPayload) {
    if let Some(count) = payload.count {
        *flags |= FLAG_COUNT;
        out.extend_from_slice(&count.to_le_bytes());
    }
    if let Some(scale) = payload.scale {
        *flags |= FLAG_SCALE;
        push_f32(out, scale);
    }
    if let Some(speed) = payload.speed {
        *flags |= FLAG_SPEED;
        push_f32(out, speed);
    }
    if let Some(tint) = payload.tint {
        *flags |= FLAG_TINT;
        for c in tint {
            out.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    if let Some(dir) = payload.dir {
        *flags |= FLAG_DIR;
        for v in dir {
            push_f32(out, v);
        }
    }
    match payload.sprite {
        Some(SpriteOverride::Rect { uv }) => {
            *flags |= FLAG_SPRITE;
            for v in uv {
                push_f32(out, v);
            }
        }
        Some(SpriteOverride::Frame { index }) => {
            *flags |= FLAG_SPRITE | FLAG_SPRITE_FRAME;
            out.extend_from_slice(&index.to_le_bytes());
        }
        None => {}
    }
    if let Some(texture) = &payload.texture {
        *flags |= FLAG_TEXTURE;
        // Same length convention as the effect name: `u8` len, truncated.
        let bytes = texture.as_bytes();
        out.push(bytes.len().min(255) as u8);
        out.extend_from_slice(&bytes[..bytes.len().min(255)]);
    }
}

impl ParticlesEmitRpc {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + self.effect.len());
        let mut flags = 0u8;
        let effect = self.effect.as_bytes();
        // Effect name length is deferred until after the flag byte, so encode
        // the body into a scratch buffer first.
        let mut body = Vec::with_capacity(effect.len() + 24);
        body.push(effect.len().min(255) as u8);
        body.extend_from_slice(&effect[..effect.len().min(255)]);
        for v in self.pos {
            push_f32(&mut body, v);
        }
        for v in self.vel {
            push_f32(&mut body, v);
        }
        encode_payload_fields(&mut body, &mut flags, &self.payload);
        out.push(flags);
        out.extend_from_slice(&body);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor {
            data: payload,
            pos: 0,
        };
        let flags = c.byte()?;
        let effect = c.string()?;
        let pos = [c.f32()?, c.f32()?, c.f32()?];
        let vel = [c.f32()?, c.f32()?, c.f32()?];
        let payload = SpawnPayload {
            count: if flags & FLAG_COUNT != 0 {
                Some(c.u32()?)
            } else {
                None
            },
            scale: if flags & FLAG_SCALE != 0 {
                Some(c.f32()?)
            } else {
                None
            },
            speed: if flags & FLAG_SPEED != 0 {
                Some(c.f32()?)
            } else {
                None
            },
            tint: if flags & FLAG_TINT != 0 {
                Some([
                    c.byte()? as f32 / 255.0,
                    c.byte()? as f32 / 255.0,
                    c.byte()? as f32 / 255.0,
                    c.byte()? as f32 / 255.0,
                ])
            } else {
                None
            },
            dir: if flags & FLAG_DIR != 0 {
                Some([c.f32()?, c.f32()?, c.f32()?])
            } else {
                None
            },
            sprite: if flags & FLAG_SPRITE != 0 {
                if flags & FLAG_SPRITE_FRAME != 0 {
                    Some(SpriteOverride::Frame { index: c.u32()? })
                } else {
                    Some(SpriteOverride::Rect {
                        uv: [c.f32()?, c.f32()?, c.f32()?, c.f32()?],
                    })
                }
            } else {
                None
            },
            texture: if flags & FLAG_TEXTURE != 0 {
                Some(c.string()?)
            } else {
                None
            },
        };
        Some(Self {
            effect,
            pos,
            vel,
            payload,
        })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(1 + self.effect.len() + 40);
        p.push(SVC::ParticlesEmit as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

// `particles_emit` moved to `ServerThreadCtx` (deliver_s2c_broadcast with
// UDP happens on the sim thread).

/// Deferred `sv_particles_test` emits (console runs before the server tick).
static PENDING_SV_EMIT: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

/// Register the particle debug console commands (client + server test bursts).
pub fn register_commands(cmd: &mut CmdSystem) {
    #[cfg(any(feature = "client", feature = "full"))]
    cmd.register(CommandRegistration::new("particles_test").execute(|ctx| {
        let Some(effect) = ctx.args.first().copied() else {
            log::info!("Usage: particles_test <effect> [count]");
            return;
        };
        let count = ctx.args.get(1).and_then(|s| s.parse::<u32>().ok());
        let Some(client) = ctx.cmd.client_engine_mut() else {
            log::warn!("[particles_test] no client");
            return;
        };
        // Above the eye, outside the near-plane slab: quads spawned AT the
        // camera position are entirely closer than `near` and never raster.
        let [ex, ey, ez] = client.particle_camera_pos(None).to_array();
        let pos = [ex, ey, ez + 2.5];
        let payload = SpawnPayload {
            count,
            ..SpawnPayload::default()
        };
        client.spawn_particles_burst(effect, pos, [0.0, 0.0, 0.0], &payload);
    }));
    cmd.register(
        CommandRegistration::new("sv_particles_test").execute(|ctx| {
            let Some(effect) = ctx.args.first().copied() else {
                log::info!("Usage: sv_particles_test <effect> [count]");
                return;
            };
            let count = ctx.args.get(1).and_then(|s| s.parse::<u32>().ok());
            PENDING_SV_EMIT
                .lock()
                .unwrap()
                .push((effect.to_string(), count.unwrap_or(0)));
        }),
    );
}

/// Server tick: flush deferred `sv_particles_test` emits through the svc path.
pub fn drain_pending_sv_emits(ctx: &mut crate::server_ctx::ServerThreadCtx) {
    let pending = std::mem::take(&mut *PENDING_SV_EMIT.lock().unwrap());
    for (effect, count) in pending {
        let payload = SpawnPayload {
            count: (count > 0).then_some(count),
            ..SpawnPayload::default()
        };
        ctx.particles_emit(&effect, [0.0, 2.0, 0.0], [0.0, 0.0, 0.0], &payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(rpc: ParticlesEmitRpc) {
        let decoded = ParticlesEmitRpc::decode(&rpc.encode()).expect("decode");
        assert_eq!(decoded.effect, rpc.effect);
        assert_eq!(decoded.pos, rpc.pos);
        assert_eq!(decoded.vel, rpc.vel);
        assert_eq!(decoded.payload, rpc.payload);
    }

    #[test]
    fn roundtrip_empty_payload() {
        roundtrip(ParticlesEmitRpc {
            effect: "block_dust".into(),
            pos: [1.5, 2.0, -3.25],
            vel: [0.0, -1.0, 0.0],
            payload: SpawnPayload::default(),
        });
    }

    #[test]
    fn roundtrip_full_payload() {
        roundtrip(ParticlesEmitRpc {
            effect: "x".into(),
            pos: [0.0; 3],
            vel: [1.0; 3],
            payload: SpawnPayload {
                count: Some(42),
                scale: Some(2.5),
                speed: Some(0.75),
                tint: Some([64.0 / 255.0, 128.0 / 255.0, 1.0, 0.0]),
                dir: Some([0.0, 1.0, 0.0]),
                sprite: Some(SpriteOverride::Rect {
                    uv: [0.0, 0.0, 0.5, 0.5],
                }),
                texture: Some("textures/particles/leaf_oak.png".into()),
            },
        });
    }

    #[test]
    fn roundtrip_frame_sprite() {
        roundtrip(ParticlesEmitRpc {
            effect: "y".into(),
            pos: [10.0, 20.0, 30.0],
            vel: [0.0; 3],
            payload: SpawnPayload {
                sprite: Some(SpriteOverride::Frame { index: 7 }),
                ..SpawnPayload::default()
            },
        });
    }

    #[test]
    fn tint_quantizes_to_bytes() {
        let rpc = ParticlesEmitRpc {
            effect: "t".into(),
            pos: [0.0; 3],
            vel: [0.0; 3],
            payload: SpawnPayload {
                tint: Some([1.0, 0.0, 0.50196078, 0.003_921_6]),
                ..SpawnPayload::default()
            },
        };
        let decoded = ParticlesEmitRpc::decode(&rpc.encode()).unwrap();
        let tint = decoded.payload.tint.unwrap();
        assert!((tint[0] - 1.0).abs() < 1.0 / 255.0);
        assert!(tint[1].abs() < 1.0 / 255.0);
        // ~128/255, not exactly the source float.
        assert!((tint[2] - 0.50196078).abs() <= 0.5 / 255.0);
    }

    #[test]
    fn decode_truncated_fails() {
        let full = ParticlesEmitRpc {
            effect: "z".into(),
            pos: [1.0; 3],
            vel: [0.0; 3],
            payload: SpawnPayload::default(),
        }
        .encode();
        for n in 0..full.len() {
            assert!(ParticlesEmitRpc::decode(&full[..n]).is_none());
        }
    }

    #[test]
    fn packet_starts_with_svc_cmd() {
        let pkt = ParticlesEmitRpc {
            effect: "e".into(),
            pos: [0.0; 3],
            vel: [0.0; 3],
            payload: SpawnPayload::default(),
        }
        .to_packet();
        assert_eq!(pkt[0], SVC::ParticlesEmit as u8);
    }
}
