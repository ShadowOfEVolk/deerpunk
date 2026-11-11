use std::sync::Arc;

use deerpunk_engine_common::particle_gpu::{GpuParticle, ParticleFrame, GPU_PARTICLE_STRIDE};
use glam::Vec3;

use crate::affector::{Affector, AffectorRegistry, Rest};
use crate::collider::{ColliderHit, WorldCollider};
use crate::emitter::{atlas_uv_rect, Sample, SpawnPayload, SpriteOverride};
use crate::format::{
    AffectorDef, ColorDef, EffectDef, EmitterDef, EmitterKind, OneOrRange, ParticleDef,
    SpriteSource,
};
use crate::particle::ParticleState;
use crate::pool::ParticlePool;
use crate::sim::{ParticleSim, Rng};

fn reg() -> AffectorRegistry {
    AffectorRegistry::new()
}

fn test_def() -> EffectDef {
    EffectDef {
        sprite: SpriteSource::Texture {
            texture: "textures/particles/smoke.png".into(),
        },
        emitter: EmitterDef {
            kind: EmitterKind::Burst,
            count: OneOrRange::Single(4.0),
            rate: 10.0,
            cone_deg: 0.0,
            dir: [0.0, 0.0, 1.0],
            speed: OneOrRange::Single(2.0),
            spread: 0.0,
            inherit_velocity: 0.0,
        },
        particle: ParticleDef {
            ttl: OneOrRange::Single(1.0),
            size: OneOrRange::Single(0.1),
            spin: 0.0,
            opacity: 1.0,
            fade_out: 0.25,
            color: ColorDef {
                start: [1.0; 4],
                end: [1.0; 4],
            },
        },
        affectors: vec![],
        quota: 512,
    }
}

fn test_particle() -> crate::Particle {
    crate::Particle {
        pos: Vec3::ZERO,
        vel: Vec3::ZERO,
        age: 0.0,
        ttl: 1.0,
        size: 0.1,
        spin: 0.0,
        rot: 0.0,
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        tex_uri: "textures/particles/smoke.png".into(),
        tint_rgb: [1.0; 3],
        tint_a: 1.0,
        grad: None,
        opacity: 1.0,
        fade_out: 0.0,
        tag: 0,
        state: ParticleState::Air,
        phase: 0.0,
        affectors: Arc::from(Vec::new()),
    }
}

// --- format parsing ---

#[test]
fn parses_all_builtin_affectors() {
    let src = r#"{
        "sprite": {"texture": "a.png"},
        "emitter": {"kind": "burst", "count": 8},
        "particle": {"ttl": 1.0},
        "affectors": [
            {"type": "gravity", "g": 22.0},
            {"type": "drag", "k": 0.4},
            {"type": "terminal_velocity", "v": 40.0},
            {"type": "sway", "freq": 1.0, "amp": 0.3},
            {"type": "flutter", "amp": 0.2, "freq": 2.0},
            {"type": "spin_wobble", "wobble": 3.0, "freq": 1.5},
            {"type": "linear_force", "f": [1.0, 2.0, 3.0]},
            {"type": "collide_world", "bounce": 0.3, "friction": 0.6, "rest": "sleep"}
        ]
    }"#;
    let def = EffectDef::parse(src, &reg(), true).expect("full def parses");
    assert_eq!(def.affectors.len(), 8);
    let resolved: Vec<Affector> = def
        .affectors
        .iter()
        .cloned()
        .map(|a| a.resolve(&reg()).unwrap())
        .collect();
    assert_eq!(resolved[0], Affector::Gravity { g: 22.0 });
    assert_eq!(resolved[1], Affector::Drag { k: 0.4 });
    assert_eq!(resolved[2], Affector::TerminalVelocity { v: 40.0 });
    assert_eq!(
        resolved[3],
        Affector::Sway {
            freq: 1.0,
            amp: 0.3
        }
    );
    assert_eq!(resolved[6], Affector::LinearForce { f: [1.0, 2.0, 3.0] });
    assert_eq!(
        resolved[7],
        Affector::CollideWorld {
            bounce: 0.3,
            friction: 0.6,
            rest: Rest::Sleep
        }
    );
}

#[test]
fn collide_world_without_collider_is_hard_error() {
    let src = r#"{
        "sprite": {"texture": "a.png"},
        "emitter": {"kind": "burst"},
        "particle": {"ttl": 1.0},
        "affectors": [{"type": "collide_world"}]
    }"#;
    let err = EffectDef::parse(src, &reg(), false).unwrap_err();
    assert!(err.contains("collide_world"), "got: {err}");
}

#[test]
fn hex_colors_flat_and_gradient() {
    assert_eq!(
        crate::format::parse_hex("#FF8000").unwrap(),
        [1.0, 128.0 / 255.0, 0.0, 1.0]
    );
    assert_eq!(
        crate::format::parse_hex("#10203040").unwrap(),
        [16.0 / 255.0, 32.0 / 255.0, 48.0 / 255.0, 64.0 / 255.0]
    );
    assert!(crate::format::parse_hex("#12345").is_err());
    let src = r##"{
        "sprite": {"texture": "a.png"},
        "emitter": {"kind": "burst"},
        "particle": {"ttl": 1.0, "color": {"start": "#FF0000", "end": "#0000FF80"}}
    }"##;
    let def = EffectDef::parse(src, &reg(), false).unwrap();
    assert_eq!(def.particle.color.start, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(def.particle.color.end, [0.0, 0.0, 1.0, 128.0 / 255.0]);
    let mid = def.particle.color.eval(0.5);
    assert!((mid[0] - 0.5).abs() < 1e-5);
}

#[test]
fn custom_affector_passthrough_and_unknown_hard_error() {
    let mut r = reg();
    let src_ok = r#"{
        "sprite": {"texture": "a.png"},
        "emitter": {"kind": "burst"},
        "particle": {"ttl": 1.0},
        "affectors": [{"type": "ohb:sparks", "streaks": 3}]
    }"#;
    // Without registration: hard error.
    let err = EffectDef::parse(src_ok, &r, false).unwrap_err();
    assert!(err.contains("ohb:sparks"), "got: {err}");
    // With registration: params kept verbatim.
    r.register_custom("ohb:sparks", std::sync::Arc::new(NoopCustom));
    let def = EffectDef::parse(src_ok, &r, false).unwrap();
    match def.affectors[0].clone().resolve(&r).unwrap() {
        Affector::Custom { type_name, params } => {
            assert_eq!(type_name, "ohb:sparks");
            assert_eq!(params.get("streaks").and_then(|v| v.as_i64()), Some(3));
        }
        other => panic!("expected custom, got {other:?}"),
    }
}

struct NoopCustom;
impl crate::affector::CustomAffector for NoopCustom {
    fn apply(&self, _p: &mut crate::Particle, _dt: f32, _params: &serde_json::Value) {}
}

#[test]
fn procedural_sprite_defers_with_clear_error() {
    let src = r#"{
        "sprite": {"procedural": "soft_disc"},
        "emitter": {"kind": "burst"},
        "particle": {"ttl": 1.0}
    }"#;
    let err = EffectDef::parse(src, &reg(), false).unwrap_err();
    assert!(err.contains("soft_disc"), "got: {err}");
}

// --- payload overrides ---

#[test]
fn payload_none_falls_back_to_template() {
    let def = test_def();
    let payload = SpawnPayload::default();
    let s = Sample {
        emitter: &def.emitter,
        particle: &def.particle,
        payload: &payload,
    };
    assert_eq!(s.count(0.5), 4);
    assert_eq!(s.speed(0.5), 2.0);
    assert_eq!(s.size(0.5), 0.1);
    assert_eq!(s.start_tint(), [1.0; 4]);
}

#[test]
fn payload_overrides_apply() {
    let def = test_def();
    let payload = SpawnPayload {
        count: Some(1),
        scale: Some(2.0),
        speed: Some(3.0),
        tint: Some([0.5, 0.5, 0.5, 1.0]),
        dir: Some([0.0, 1.0, 0.0]),
        sprite: Some(SpriteOverride::Rect {
            uv: [0.1, 0.2, 0.3, 0.4],
        }),
        texture: None,
    };
    let s = Sample {
        emitter: &def.emitter,
        particle: &def.particle,
        payload: &payload,
    };
    assert_eq!(s.count(0.9), 1);
    assert_eq!(s.speed(0.0), 6.0);
    assert_eq!(s.size(0.0), 0.2);
    assert_eq!(s.start_tint(), [0.5, 0.5, 0.5, 1.0]);
    let d = s.dir(0.0, 0.0, 0.0);
    assert!(
        (d.y - 1.0).abs() < 1e-5,
        "payload dir override applied: {d}"
    );
}

#[test]
fn payload_roundtrips_through_json() {
    let p = SpawnPayload {
        count: Some(3),
        tint: Some([1.0, 0.0, 0.0, 1.0]),
        sprite: Some(SpriteOverride::Frame { index: 2 }),
        ..Default::default()
    };
    let s = serde_json::to_string(&p).unwrap();
    let back: SpawnPayload = serde_json::from_str(&s).unwrap();
    assert_eq!(back, p);
}

// --- emitter tick math ---

#[test]
fn burst_count_matches_template_and_quota() {
    let mut def = test_def();
    def.emitter.count = OneOrRange::Range([10.0, 20.0]);
    def.quota = 12;
    let mut sim = ParticleSim::new(def, &reg(), false).unwrap();
    let mut pool = ParticlePool::new(1024);
    sim.spawn_burst(
        &mut pool,
        Vec3::ZERO,
        Vec3::ZERO,
        &SpawnPayload::default(),
        0.0,
    );
    assert!(
        (10..=20).contains(&sim.live(&pool)),
        "burst count {} outside [10,20]",
        sim.live(&pool)
    );
    // Quota caps a second burst.
    sim.spawn_burst(
        &mut pool,
        Vec3::ZERO,
        Vec3::ZERO,
        &SpawnPayload::default(),
        0.0,
    );
    assert_eq!(sim.live(&pool), 12);
    assert_eq!(sim.spawned as usize, sim.live(&pool));
}

#[test]
fn cone_sampling_stays_within_half_angle() {
    let mut def = test_def();
    def.emitter.cone_deg = 35.0;
    let s = Sample {
        emitter: &def.emitter,
        particle: &def.particle,
        payload: &SpawnPayload::default(),
    };
    let cos_limit = 35.0f32.to_radians() * 0.5;
    for i in 0..2000u32 {
        let mut rng = Rng(i as u64 | 1);
        let d = s.dir(rng.next_f32(), rng.next_f32(), rng.next_f32());
        assert!((d.length() - 1.0).abs() < 1e-4);
        let ang = d.angle_between(Vec3::Z);
        assert!(
            ang <= cos_limit + 1e-3,
            "sample {} deg outside cone",
            ang.to_degrees()
        );
    }
    // Zero cone → exact template dir.
    def.emitter.cone_deg = 0.0;
    let s = Sample {
        emitter: &def.emitter,
        particle: &def.particle,
        payload: &SpawnPayload::default(),
    };
    let d = s.dir(0.0, 0.0, 0.0);
    assert!((d.z - 1.0).abs() < 1e-5);
}

#[test]
fn stream_owed_accumulates_rate() {
    let mut def = test_def();
    def.emitter.kind = EmitterKind::Stream;
    def.emitter.rate = 10.0;
    let mut sim = ParticleSim::new(def, &reg(), false).unwrap();
    assert_eq!(sim.stream_owed(0.05), 0); // 0.5 owed, kept fractional
    assert_eq!(sim.stream_owed(0.05), 1);
    assert_eq!(sim.stream_owed(1.0), 10);
}

// --- integrate + collide_world ---

struct FloorCollider;
impl WorldCollider for FloorCollider {
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<ColliderHit> {
        // z = 0 plane.
        if dir.z >= -1e-6 || origin.z <= 0.0 {
            return None;
        }
        let t = -origin.z / dir.z;
        if t > max_dist {
            return None;
        }
        Some(ColliderHit {
            point: origin + dir * t,
            normal: Vec3::Z,
            dist: t,
        })
    }
}

fn gravity_collide_def() -> EffectDef {
    let mut def = test_def();
    def.emitter.speed = OneOrRange::Single(0.0);
    def.particle.ttl = OneOrRange::Single(10.0);
    def.affectors = vec![
        AffectorDef::Gravity { g: 10.0 },
        AffectorDef::CollideWorld {
            bounce: 0.0,
            friction: 0.5,
            rest: "sleep".into(),
        },
    ];
    def
}

#[test]
fn integrate_collides_and_rests() {
    let mut sim = ParticleSim::new(gravity_collide_def(), &reg(), true).unwrap();
    let mut pool = ParticlePool::new(64);
    pool.set_collider(Some(Arc::new(FloorCollider)));
    let mut payload = SpawnPayload::default();
    payload.dir = Some([0.0, 0.0, -1.0]);
    payload.count = Some(1);
    sim.spawn_burst(
        &mut pool,
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::ZERO,
        &payload,
        0.0,
    );
    assert_eq!(sim.live(&pool), 1);
    // Step until the particle hits the floor and settles.
    for _ in 0..200 {
        pool.integrate(1.0 / 60.0);
        if pool.particles()[0].state == ParticleState::Rest {
            break;
        }
    }
    let p = &pool.particles()[0];
    assert_eq!(p.state, ParticleState::Rest, "particle should settle");
    assert!(p.pos.z.abs() < 1e-3, "rests on floor, z={}", p.pos.z);
    assert_eq!(p.vel.length(), 0.0);
    let z_rest = p.pos.z;
    pool.integrate(1.0 / 60.0);
    assert!(
        (pool.particles()[0].pos.z - z_rest).abs() < 1e-5,
        "rest particles don't move"
    );
    // ttl expires eventually.
    for _ in 0..700 {
        pool.integrate(1.0 / 60.0);
    }
    assert_eq!(sim.live(&pool), 0);
}

#[test]
fn rest_stop_despawns_on_landing() {
    let mut def = gravity_collide_def();
    def.affectors[1] = AffectorDef::CollideWorld {
        bounce: 0.0,
        friction: 0.5,
        rest: "stop".into(),
    };
    let mut sim = ParticleSim::new(def, &reg(), true).unwrap();
    let mut pool = ParticlePool::new(64);
    pool.set_collider(Some(Arc::new(FloorCollider)));
    let mut payload = SpawnPayload::default();
    payload.dir = Some([0.0, 0.0, -1.0]);
    sim.spawn_burst(
        &mut pool,
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::ZERO,
        &payload,
        0.0,
    );
    for _ in 0..200 {
        pool.integrate(1.0 / 60.0);
        if pool.stats().0 == 0 {
            break;
        }
    }
    assert_eq!(pool.stats().0, 0, "stop-rest particles despawn on landing");
}

#[test]
fn payload_sprite_override_rides_on_particles() {
    let mut def = test_def();
    def.emitter.count = OneOrRange::Single(2.0);
    let mut sim = ParticleSim::new(def, &reg(), false).unwrap();
    // Rect: every particle carries the exact game-resolved rect (≠ def default full rect).
    let payload = SpawnPayload {
        sprite: Some(SpriteOverride::Rect {
            uv: [0.1, 0.2, 0.3, 0.4],
        }),
        count: Some(2),
        ..Default::default()
    };
    let mut pool = ParticlePool::new(64);
    sim.spawn_burst(&mut pool, Vec3::ZERO, Vec3::ZERO, &payload, 0.0);
    assert_eq!(sim.live(&pool), 2);
    for p in pool.particles() {
        assert_eq!(p.uv_rect, [0.1, 0.2, 0.3, 0.4], "Rect override verbatim");
    }
    // Frame: tile=2 atlas, frame 3 → row-major rect.
    let mut sim2 = ParticleSim::new(
        {
            let mut d = test_def();
            d.sprite = SpriteSource::Atlas {
                atlas: "atlas.png".into(),
                tile: 2,
            };
            d
        },
        &reg(),
        false,
    )
    .unwrap();
    let payload = SpawnPayload {
        sprite: Some(SpriteOverride::Frame { index: 3 }),
        count: Some(1),
        ..Default::default()
    };
    let mut pool2 = ParticlePool::new(64);
    sim2.spawn_burst(&mut pool2, Vec3::ZERO, Vec3::ZERO, &payload, 0.0);
    assert_eq!(pool2.particles()[0].uv_rect, atlas_uv_rect(2, 3));
    // No override, static texture → resolved to the full rect.
    let mut sim3 = ParticleSim::new(test_def(), &reg(), false).unwrap();
    let mut pool3 = ParticlePool::new(64);
    sim3.spawn_burst(
        &mut pool3,
        Vec3::ZERO,
        Vec3::ZERO,
        &SpawnPayload::default(),
        0.0,
    );
    assert_eq!(pool3.particles()[0].uv_rect, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn payload_texture_override_replaces_bound_texture() {
    // Atlas effect (tile=2): without an explicit rect, the override texture
    // samples the full rect — no atlas-frame slicing of the wrong texture.
    let mut sim = ParticleSim::new(
        {
            let mut d = test_def();
            d.sprite = SpriteSource::Atlas {
                atlas: "atlas.png".into(),
                tile: 2,
            };
            d
        },
        &reg(),
        false,
    )
    .unwrap();
    let payload = SpawnPayload {
        texture: Some("textures/particles/leaf_oak.png".into()),
        count: Some(1),
        ..Default::default()
    };
    let mut pool = ParticlePool::new(64);
    sim.spawn_burst(&mut pool, Vec3::ZERO, Vec3::ZERO, &payload, 0.0);
    assert_eq!(sim.live(&pool), 1);
    let p = &pool.particles()[0];
    assert_eq!(p.tex_uri.as_ref(), "textures/particles/leaf_oak.png");
    assert_eq!(
        p.uv_rect,
        [0.0, 0.0, 1.0, 1.0],
        "full rect over the override"
    );

    // Explicit rect override rides along: UVs address the override texture.
    let payload = SpawnPayload {
        texture: Some("textures/particles/leaf_oak.png".into()),
        sprite: Some(SpriteOverride::Rect {
            uv: [0.25, 0.25, 0.75, 0.75],
        }),
        count: Some(1),
        ..Default::default()
    };
    let mut pool2 = ParticlePool::new(64);
    sim.spawn_burst(&mut pool2, Vec3::ZERO, Vec3::ZERO, &payload, 0.0);
    let p = &pool2.particles()[0];
    assert_eq!(p.tex_uri.as_ref(), "textures/particles/leaf_oak.png");
    assert_eq!(p.uv_rect, [0.25, 0.25, 0.75, 0.75]);

    // No override → the effect's bound sprite texture (fresh sim: frame 0).
    let mut sim2 = ParticleSim::new(
        {
            let mut d = test_def();
            d.sprite = SpriteSource::Atlas {
                atlas: "atlas.png".into(),
                tile: 2,
            };
            d
        },
        &reg(),
        false,
    )
    .unwrap();
    let mut pool3 = ParticlePool::new(64);
    sim2.spawn_burst(
        &mut pool3,
        Vec3::ZERO,
        Vec3::ZERO,
        &SpawnPayload::default(),
        0.0,
    );
    assert_eq!(pool3.particles()[0].tex_uri.as_ref(), "atlas.png");
    // Per-particle frame is RNG-driven; any 2×2 atlas rect is valid.
    let uv = pool3.particles()[0].uv_rect;
    let frame = (uv[1] * 2.0) as usize * 2 + (uv[0] * 2.0) as usize;
    assert_eq!(uv, atlas_uv_rect(2, frame));
}

#[test]
fn atlas_uv_rect_frames() {
    assert_eq!(atlas_uv_rect(1, 0), [0.0, 0.0, 1.0, 1.0]);
    let r = atlas_uv_rect(3, 4);
    assert_eq!(r, [1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]);
    assert_eq!(atlas_uv_rect(3, 99), atlas_uv_rect(3, 0)); // wraps
}

// --- pool: capacity, compaction, packing ---

#[test]
fn gpu_particle_layout_matches_stride() {
    assert_eq!(
        std::mem::size_of::<GpuParticle>() as u32,
        GPU_PARTICLE_STRIDE
    );
    // center[12] half[4] rot[4] uv_rect[16] tint[4] — no implicit padding.
    assert_eq!(
        std::mem::offset_of!(GpuParticle, tint) as u32,
        GPU_PARTICLE_STRIDE - 4
    );
}

#[test]
fn pool_spawn_clamps_at_capacity() {
    let mut pool = ParticlePool::new(3);
    for _ in 0..3 {
        assert!(pool.spawn(test_particle()), "spawn within capacity");
    }
    assert!(!pool.spawn(test_particle()), "full pool refuses spawn");
    assert_eq!(pool.stats(), (3, 3));
    // clear() drops everything, keeps the allocation.
    pool.clear();
    assert_eq!(pool.stats(), (0, 3));
    assert!(pool.spawn(test_particle()));
}

#[test]
fn pool_integrate_kills_expired_and_compacts() {
    let mut pool = ParticlePool::new(8);
    for _ in 0..3 {
        let mut p = test_particle();
        p.ttl = 0.05;
        assert!(pool.spawn(p));
    }
    let mut long = test_particle();
    long.ttl = 10.0;
    assert!(pool.spawn(long));
    pool.integrate(0.1);
    assert_eq!(pool.stats(), (1, 8), "expired particles removed");
    let ps = pool.particles();
    assert_eq!(ps.len(), 1);
    assert!(ps[0].ttl > 1.0, "long-lived survivor remains (swap-remove)");
    assert!(
        ps.iter().all(|p| p.age < p.ttl),
        "pool stays dense: only live particles"
    );
}

#[test]
fn pool_tag_counts_track_liveness() {
    let mut pool = ParticlePool::new(4);
    for _ in 0..2 {
        let mut p = test_particle();
        p.tag = 77;
        p.ttl = 0.05;
        assert!(pool.spawn(p));
    }
    assert_eq!(pool.live_for_tag(77), 2);
    assert_eq!(pool.live_for_tag(78), 0);
    pool.integrate(0.1);
    assert_eq!(pool.live_for_tag(77), 0, "death decrements the tag count");
}

#[test]
fn pack_groups_by_texture_in_first_seen_order() {
    let mut pool = ParticlePool::new(8);
    // tex a, a, b, a, b → buckets a(3) then b(2), instances contiguous.
    let uris = ["a.png", "a.png", "b.png", "a.png", "b.png"];
    for uri in uris {
        let mut p = test_particle();
        p.tex_uri = uri.into();
        assert!(pool.spawn(p));
    }
    let mut frame = ParticleFrame::default();
    pool.pack_into(&mut frame);
    assert_eq!(frame.ranges.len(), 2);
    assert_eq!(frame.ranges[0].tex_uri.as_ref(), "a.png");
    assert_eq!(frame.ranges[0].offset, 0);
    assert_eq!(frame.ranges[0].len, 3);
    assert_eq!(frame.ranges[1].tex_uri.as_ref(), "b.png");
    assert_eq!(frame.ranges[1].offset, 3);
    assert_eq!(frame.ranges[1].len, 2);
    assert_eq!(frame.instances.len(), 5);
    // Repacking clears the frame instead of appending.
    pool.pack_into(&mut frame);
    assert_eq!(frame.instances.len(), 5);
    assert_eq!(frame.ranges.len(), 2);
    pool.clear();
    pool.pack_into(&mut frame);
    assert!(frame.instances.is_empty() && frame.ranges.is_empty());
}

#[test]
fn pack_alpha_fades_with_opacity_and_fade_out() {
    let mut pool = ParticlePool::new(2);
    let mut p = test_particle();
    p.ttl = 1.0;
    p.opacity = 0.5;
    p.fade_out = 0.25;
    assert!(pool.spawn(p));
    let mut frame = ParticleFrame::default();
    pool.pack_into(&mut frame);
    // Fresh particle: alpha = tint_a × opacity = 0.5 → 128 (rounds 127.5 up).
    assert_eq!(frame.instances[0].tint, [255, 255, 255, 128]);
    // Past the fade start (t=0.9 > 0.75): alpha = 0.5 × (1 - 0.6) = 0.2 → 51.
    pool.integrate(0.9);
    let mut frame = ParticleFrame::default();
    pool.pack_into(&mut frame);
    assert_eq!(frame.instances[0].tint, [255, 255, 255, 51]);
}

#[test]
fn pack_gradient_color_over_lifetime() {
    let mut pool = ParticlePool::new(2);
    let mut p = test_particle();
    p.ttl = 1.0;
    p.opacity = 1.0;
    p.fade_out = 0.0;
    p.grad = Some(ColorDef {
        start: [1.0, 0.0, 0.0, 1.0],
        end: [0.0, 0.0, 1.0, 0.5],
    });
    assert!(pool.spawn(p));
    // Mid-life t=0.5: rgb lerps to [0.5, 0, 0.5], alpha to 0.75.
    pool.integrate(0.5);
    let mut frame = ParticleFrame::default();
    pool.pack_into(&mut frame);
    assert_eq!(frame.instances[0].tint, [128, 0, 128, 191]);
}
