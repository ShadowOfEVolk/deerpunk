//! Datapack / JSON sound event definitions (Kira and tools).

use std::collections::HashMap;

use serde::Deserialize;

/// Merged registry of logical event names → definition.
#[derive(Clone, Debug, Default)]
pub struct SoundEventRegistry {
    pub events: HashMap<String, SoundEventDef>,
}

impl SoundEventRegistry {
    pub fn merge_bank(&mut self, bank: SoundEventBank) {
        for (k, v) in bank.events {
            self.events.insert(k, v);
        }
    }

    pub fn insert(&mut self, id: impl Into<String>, def: SoundEventDef) {
        self.events.insert(id.into(), def);
    }
}

/// `sound_banks/*.json` — many events in one file.
#[derive(Clone, Debug, Deserialize)]
pub struct SoundEventBank {
    #[serde(default)]
    pub events: HashMap<String, SoundEventDef>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SoundEventDef {
    /// One clip.
    Single {
        /// `file://…` or datapack-relative (`sounds/foo.ogg`).
        clip: String,
        #[serde(default = "default_volume")]
        volume: f32,
        #[serde(default)]
        pitch: PitchRange,
        #[serde(default)]
        params: HashMap<String, SoundParamMap>,
        #[serde(default)]
        r#loop: bool,
    },
    /// Pick a clip at random each play.
    Random {
        clips: Vec<String>,
        #[serde(default = "default_volume")]
        volume: f32,
        #[serde(default)]
        pitch: PitchRange,
        #[serde(default)]
        params: HashMap<String, SoundParamMap>,
        #[serde(default)]
        r#loop: bool,
    },
}

fn default_volume() -> f32 {
    1.0
}

/// Pitch multiplier range (`1.0` = unchanged).
#[derive(Clone, Copy, Debug)]
pub struct PitchRange {
    pub min: f32,
    pub max: f32,
}

impl Default for PitchRange {
    fn default() -> Self {
        Self { min: 1.0, max: 1.0 }
    }
}

impl<'de> Deserialize<'de> for PitchRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Rep {
            Scalar(f32),
            Pair([f32; 2]),
            Obj { min: f32, max: f32 },
        }
        match Rep::deserialize(deserializer)? {
            Rep::Scalar(v) => Ok(Self { min: v, max: v }),
            Rep::Pair([a, b]) => Ok(Self { min: a, max: b }),
            Rep::Obj { min, max } => Ok(Self { min, max }),
        }
    }
}

/// Maps a gameplay parameter (e.g. `intensity`) to volume/pitch scaling.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct SoundParamMap {
    /// `volume *= lerp(min, max, param)` when param is in 0..1.
    #[serde(default)]
    pub volume: Option<ParamLerp>,
    #[serde(default)]
    pub pitch: Option<ParamLerp>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct ParamLerp {
    pub min: f32,
    pub max: f32,
}

impl SoundEventDef {
    pub fn is_loop(&self) -> bool {
        match self {
            Self::Single { r#loop, .. } => *r#loop,
            Self::Random { r#loop, .. } => *r#loop,
        }
    }

    pub fn pick_clip<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> Option<&str> {
        match self {
            Self::Single { clip, .. } => Some(clip.as_str()),
            Self::Random { clips, .. } => {
                if clips.is_empty() {
                    None
                } else {
                    let i = rng.gen_range(0..clips.len());
                    Some(clips[i].as_str())
                }
            }
        }
    }

    pub fn base_volume(&self) -> f32 {
        match self {
            Self::Single { volume, .. } | Self::Random { volume, .. } => *volume,
        }
    }

    pub fn pitch_range(&self) -> PitchRange {
        match self {
            Self::Single { pitch, .. } | Self::Random { pitch, .. } => *pitch,
        }
    }

    pub fn params_map(&self) -> &HashMap<String, SoundParamMap> {
        match self {
            Self::Single { params, .. } | Self::Random { params, .. } => params,
        }
    }
}

/// Normalize event path from gameplay / network (`event:/foo` → `foo`).
pub fn normalize_event_id(event: &str) -> &str {
    event
        .strip_prefix("event:/")
        .or_else(|| event.strip_prefix("event:"))
        .unwrap_or(event)
}
