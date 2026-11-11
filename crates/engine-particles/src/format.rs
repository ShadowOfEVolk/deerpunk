//! Universal effect file format (`<gamedir>/particles/<name>.json`), engine-owned.
//!
//! Parsing is strict: unknown affector type names fail the load (they must be
//! builtin or registered as [`crate::AffectorRegistry`] customs); malformed
//! numbers/hex colors fail too. No silent defaults beyond the documented ones.

use std::path::Path;

use serde::Deserialize;

use crate::affector::{Affector, AffectorRegistry, Rest};

/// Sprite source. Exactly one variant per effect:
/// `{"texture": "path.png"}` | `{"atlas": "<uri>", "tile": 3}` | `{"procedural": "soft_disc"}`.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteSource {
    /// One full-texture sprite.
    Texture { texture: String },
    /// Tiled atlas: `tile` = tiles per row/column.
    Atlas { atlas: String, tile: u32 },
    /// Cheap generated sprite. Only `"soft_disc"` is recognized (deferred).
    Procedural(String),
}

fn default_tile() -> u32 {
    1
}

impl<'de> Deserialize<'de> for SpriteSource {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            texture: Option<String>,
            atlas: Option<String>,
            tile: Option<u32>,
            procedural: Option<String>,
        }
        let raw = Raw::deserialize(d)?;
        match (raw.texture, raw.atlas, raw.procedural) {
            (Some(texture), None, None) => Ok(Self::Texture { texture }),
            (None, Some(atlas), None) => Ok(Self::Atlas {
                atlas,
                tile: raw.tile.unwrap_or_else(default_tile),
            }),
            (None, None, Some(p)) => Ok(Self::Procedural(p)),
            _ => Err(serde::de::Error::custom(
                "sprite needs exactly one of 'texture', 'atlas' or 'procedural'",
            )),
        }
    }
}

impl SpriteSource {
    /// Texture uri the renderer must have loaded for this effect.
    pub fn texture_uri(&self) -> Option<&str> {
        match self {
            Self::Texture { texture } => Some(texture),
            Self::Atlas { atlas, .. } => Some(atlas),
            Self::Procedural(_) => None,
        }
    }
}

/// Spawn shape.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmitterKind {
    /// All `count` particles at once (one-shot).
    Burst,
    /// `rate` particles per second while enabled.
    Stream,
}

/// Scalar or `[min, max]` range (sampled per particle).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum OneOrRange {
    Single(f32),
    Range([f32; 2]),
}

impl Default for OneOrRange {
    fn default() -> Self {
        Self::Single(0.0)
    }
}

impl OneOrRange {
    pub fn sample(&self, u: f32) -> f32 {
        match *self {
            Self::Single(v) => v,
            Self::Range([a, b]) => a + (b - a) * u.clamp(0.0, 1.0),
        }
    }

    pub fn min(&self) -> f32 {
        match *self {
            Self::Single(v) => v,
            Self::Range([a, b]) => a.min(b),
        }
    }
}

/// Flat color or start→end gradient (interpolated over particle lifetime).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorDef {
    pub start: [f32; 4],
    pub end: [f32; 4],
}

impl ColorDef {
    pub fn eval(&self, t: f32) -> [f32; 4] {
        let t = t.clamp(0.0, 1.0);
        let mut out = [0.0; 4];
        for i in 0..4 {
            out[i] = self.start[i] + (self.end[i] - self.start[i]) * t;
        }
        out
    }
}

impl<'de> Deserialize<'de> for ColorDef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flat(String),
            Grad { start: String, end: String },
        }
        let raw = Raw::deserialize(d)?;
        Ok(match raw {
            Raw::Flat(s) => {
                let c = parse_hex(&s).map_err(serde::de::Error::custom)?;
                Self { start: c, end: c }
            }
            Raw::Grad { start, end } => Self {
                start: parse_hex(&start).map_err(serde::de::Error::custom)?,
                end: parse_hex(&end).map_err(serde::de::Error::custom)?,
            },
        })
    }
}

/// `#RRGGBB` or `#RRGGBBAA` (alpha defaults to 255) → linear-ish floats.
pub fn parse_hex(s: &str) -> Result<[f32; 4], String> {
    let s = s.trim().trim_start_matches('#');
    let bad = || format!("bad color '{s}': expected #RRGGBB or #RRGGBBAA");
    let hex2 = |h: &str| u8::from_str_radix(h, 16).map_err(|_| bad());
    match s.len() {
        6 => {
            let r = hex2(&s[0..2]).map_err(|_| bad())?;
            let g = hex2(&s[2..4]).map_err(|_| bad())?;
            let b = hex2(&s[4..6]).map_err(|_| bad())?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        8 => {
            let r = hex2(&s[0..2]).map_err(|_| bad())?;
            let g = hex2(&s[2..4]).map_err(|_| bad())?;
            let b = hex2(&s[4..6]).map_err(|_| bad())?;
            let a = hex2(&s[6..8]).map_err(|_| bad())?;
            Ok([
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ])
        }
        _ => Err(bad()),
    }
}

/// Emitter block.
#[derive(Clone, Debug, Deserialize)]
pub struct EmitterDef {
    pub kind: EmitterKind,
    /// Burst count, fixed or range.
    #[serde(default = "default_count")]
    pub count: OneOrRange,
    /// Stream rate (particles/second).
    #[serde(default = "default_rate")]
    pub rate: f32,
    /// Half-angle of the emission cone in degrees, around `dir`.
    #[serde(default)]
    pub cone_deg: f32,
    /// Emission direction, world space.
    #[serde(default = "default_dir")]
    pub dir: [f32; 3],
    /// Initial speed along the cone direction (range per particle).
    #[serde(default)]
    pub speed: OneOrRange,
    /// Extra radial jitter added to the base direction (0..1 fraction of speed).
    #[serde(default)]
    pub spread: f32,
    /// Fraction of the spawner's velocity added to spawned particles.
    #[serde(default)]
    pub inherit_velocity: f32,
}

fn default_count() -> OneOrRange {
    OneOrRange::Single(8.0)
}
fn default_rate() -> f32 {
    10.0
}
fn default_dir() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}

/// Particle block.
#[derive(Clone, Debug, Deserialize)]
pub struct ParticleDef {
    /// Lifetime in seconds.
    pub ttl: OneOrRange,
    /// Billboard half size in world units.
    #[serde(default = "default_size")]
    pub size: OneOrRange,
    /// Base angular velocity (rad/s).
    #[serde(default)]
    pub spin: f32,
    /// Base opacity multiplier.
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// Fraction of the lifetime at the end used to fade alpha to zero.
    #[serde(default)]
    pub fade_out: f32,
    /// Flat color or gradient over lifetime.
    #[serde(default = "default_color")]
    pub color: ColorDef,
}

fn default_size() -> OneOrRange {
    OneOrRange::Single(0.08)
}
fn default_opacity() -> f32 {
    1.0
}
fn default_color() -> ColorDef {
    ColorDef {
        start: [1.0; 4],
        end: [1.0; 4],
    }
}

/// Affector entry as written in the JSON. `"type"` selects the vocabulary;
/// unknown types pass through as `Custom` (hard-errored unless registered).
#[derive(Clone, Debug, PartialEq)]
pub enum AffectorDef {
    Gravity {
        g: f32,
    },
    Drag {
        k: f32,
    },
    TerminalVelocity {
        v: f32,
    },
    Sway {
        freq: f32,
        amp: f32,
    },
    Flutter {
        amp: f32,
        freq: f32,
    },
    SpinWobble {
        wobble: f32,
        freq: f32,
    },
    LinearForce {
        f: [f32; 3],
    },
    CollideWorld {
        bounce: f32,
        friction: f32,
        rest: String,
    },
    /// Passthrough: params kept verbatim for the registered custom affector.
    Custom(serde_json::Value),
}

fn default_bounce() -> f32 {
    0.0
}
fn default_friction() -> f32 {
    1.0
}
fn default_rest() -> String {
    "sleep".into()
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .ok_or_else(|| format!("affector missing/invalid number field '{key}'"))
}

impl<'de> Deserialize<'de> for AffectorDef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Self::from_value(v).map_err(serde::de::Error::custom)
    }
}

fn field_f32(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    num(v, key)
}

fn field_f32_or(v: &serde_json::Value, key: &str, default: f32) -> f32 {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .unwrap_or(default)
}

impl AffectorDef {
    pub fn from_value(v: serde_json::Value) -> Result<Self, String> {
        let type_name = v
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| "affector missing 'type'".to_string())?
            .to_string();
        Ok(match type_name.as_str() {
            "gravity" => Self::Gravity {
                g: field_f32(&v, "g")?,
            },
            "drag" => Self::Drag {
                k: field_f32(&v, "k")?,
            },
            "terminal_velocity" => Self::TerminalVelocity {
                v: field_f32(&v, "v")?,
            },
            "sway" => Self::Sway {
                freq: field_f32(&v, "freq")?,
                amp: field_f32(&v, "amp")?,
            },
            "flutter" => Self::Flutter {
                amp: field_f32(&v, "amp")?,
                freq: field_f32(&v, "freq")?,
            },
            "spin_wobble" => Self::SpinWobble {
                wobble: field_f32(&v, "wobble")?,
                freq: field_f32(&v, "freq")?,
            },
            "linear_force" => Self::LinearForce {
                f: v.get("f")
                    .and_then(|x| serde_json::from_value::<[f32; 3]>(x.clone()).ok())
                    .ok_or_else(|| "linear_force needs f: [x,y,z]".to_string())?,
            },
            "collide_world" => Self::CollideWorld {
                bounce: field_f32_or(&v, "bounce", default_bounce()),
                friction: field_f32_or(&v, "friction", default_friction()),
                rest: v
                    .get("rest")
                    .and_then(|x| x.as_str())
                    .map(String::from)
                    .unwrap_or_else(default_rest),
            },
            _ => Self::Custom(v),
        })
    }
}

impl AffectorDef {
    /// Resolve to a runtime [`Affector`]. Unknown `type` names hard-error
    /// unless a custom affector with that name is registered.
    pub fn resolve(self, reg: &AffectorRegistry) -> Result<Affector, String> {
        Ok(match self {
            Self::Gravity { g } => Affector::Gravity { g },
            Self::Drag { k } => Affector::Drag { k },
            Self::TerminalVelocity { v } => Affector::TerminalVelocity { v },
            Self::Sway { freq, amp } => Affector::Sway { freq, amp },
            Self::Flutter { amp, freq } => Affector::Flutter { amp, freq },
            Self::SpinWobble { wobble, freq } => Affector::SpinWobble { wobble, freq },
            Self::LinearForce { f } => Affector::LinearForce { f },
            Self::CollideWorld {
                bounce,
                friction,
                rest,
            } => {
                let rest = match rest.as_str() {
                    "stop" => Rest::Stop,
                    "sleep" => Rest::Sleep,
                    other => return Err(format!("collide_world: unknown rest '{other}'")),
                };
                Affector::CollideWorld {
                    bounce,
                    friction,
                    rest,
                }
            }
            Self::Custom(v) => {
                let type_name = v
                    .get("type")
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| "custom affector missing 'type'".to_string())?
                    .to_string();
                if reg.resolve(&type_name).is_none() {
                    return Err(format!(
                        "unknown affector type '{type_name}' (builtin or registered custom required)"
                    ));
                }
                Affector::Custom {
                    type_name,
                    params: v,
                }
            }
        })
    }
}

/// Whole effect file.
#[derive(Clone, Debug, Deserialize)]
pub struct EffectDef {
    pub sprite: SpriteSource,
    pub emitter: EmitterDef,
    pub particle: ParticleDef,
    #[serde(default)]
    pub affectors: Vec<AffectorDef>,
    /// Max live particles for this effect; extra spawns are dropped.
    #[serde(default = "default_quota")]
    pub quota: usize,
}

fn default_quota() -> usize {
    512
}

impl EffectDef {
    /// Parse and resolve one effect file. `collide_world` in the def requires
    /// `has_collider`; without it the load fails (hard error, per design).
    pub fn parse(source: &str, reg: &AffectorRegistry, has_collider: bool) -> Result<Self, String> {
        let def: Self = serde_json::from_str(source).map_err(|e| format!("effect JSON: {e}"))?;
        def.validate(reg, has_collider)?;
        Ok(def)
    }

    /// Load every `*.json` under `<gamedir>/particles/`, keyed by file stem.
    /// A malformed file is a hard error for that effect (logged loudly, effect
    /// unavailable); the rest of the catalog still loads. Returns the loaded
    /// effects plus per-file errors.
    pub fn load_catalog(
        dir: &Path,
        reg: &AffectorRegistry,
        has_collider: bool,
    ) -> Result<(Vec<(String, Self)>, Vec<String>), String> {
        let mut out = Vec::new();
        let mut errors = Vec::new();
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("particles dir {}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => {
                    errors.push(format!("bad effect file name {}", path.display()));
                    continue;
                }
            };
            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("read {}: {e}", path.display()));
                    continue;
                }
            };
            match Self::parse(&src, reg, has_collider) {
                Ok(def) => out.push((name, def)),
                Err(e) => errors.push(format!("effect '{}': {e}", path.display())),
            }
        }
        Ok((out, errors))
    }

    fn validate(&self, reg: &AffectorRegistry, has_collider: bool) -> Result<(), String> {
        if let SpriteSource::Procedural(name) = &self.sprite {
            if name != "soft_disc" {
                return Err(format!("procedural sprite '{name}' is not supported yet"));
            }
            return Err(
                "procedural sprite 'soft_disc' deferred: no generated sprite upload yet".into(),
            );
        }
        if self.emitter.kind == EmitterKind::Stream && self.emitter.rate <= 0.0 {
            return Err("stream emitter needs rate > 0".into());
        }
        if self.emitter.dir.iter().all(|&v| v == 0.0) {
            return Err("emitter dir must be non-zero".into());
        }
        if self.quota == 0 {
            return Err("quota must be > 0".into());
        }
        if self.particle.ttl.min() <= 0.0 {
            return Err("particle ttl must be > 0".into());
        }
        for aff in &self.affectors {
            if let AffectorDef::Custom(v) = aff {
                let name = v
                    .get("type")
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| "custom affector missing 'type'".to_string())?;
                if reg.resolve(name).is_none() {
                    return Err(format!("unknown affector type '{name}'"));
                }
            } else if aff_is_collide(aff) && !has_collider {
                return Err(
                    "effect uses 'collide_world' but no WorldCollider is registered".into(),
                );
            }
        }
        Ok(())
    }
}

fn aff_is_collide(a: &AffectorDef) -> bool {
    matches!(a, AffectorDef::CollideWorld { .. })
}
