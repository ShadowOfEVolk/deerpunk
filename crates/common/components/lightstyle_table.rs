//! Runtime lightstyle table (GoldSrc-style `'a'`..`'z'` patterns).

/// Number of style slots (HL / Quake).
pub const MAX_LIGHTSTYLES: usize = 64;

/// World lightstyle controller — patterns + evaluated scales.
///
/// Gamelib can change [`Self::patterns`] (switches, `light` entities);
/// call [`Self::tick`] each frame before rendering.
#[derive(Clone, Debug)]
pub struct LightstyleTable {
    /// Animation / static strings (`a`=off .. `z`=full). Empty → scale 0.
    pub patterns: Vec<String>,
    /// Evaluated scales 0..1 (length [`MAX_LIGHTSTYLES`]).
    pub scales: Vec<f32>,
}

impl Default for LightstyleTable {
    fn default() -> Self {
        Self::hl_defaults()
    }
}

/// HL / Quake standard patterns (worldspawn init). Styles 32–62 left for switchable lights.
const HL_STD_PATTERNS: &[(usize, &str)] = &[
    (0, "m"),                                                          // normal
    (1, "mmnmmommommnonmmonqnmmo"),                                     // flicker
    (2, "abcdefghijklmnopqrstuvwxyzyxwvutsrqponmlkjihgfedcba"),         // slow strong pulse
    (3, "mmmmmaaaaammmmmaaaaaabcdefgabcdefg"),                          // candle
    (4, "mamamamamama"),                                                // fast strobe
    (5, "jklmnopqrstuvwxyzyxwvutsrqponmlkj"),                           // gentle pulse
    (6, "nmonqnmomnmomomno"),                                           // flicker 2
    (7, "mmmaaaabcdefgmmmmaaaammmaamm"),                                // candle 2
    (8, "mmmaaammmaaammmabcdefaaaammmmabcdefmmmaaaa"),                  // candle 3
    (9, "aaaaaaaazzzzzzzz"),                                            // slow strobe
    (10, "mmamammmmammamamaaamammma"),                                  // fluorescent flicker
    (11, "abcdefghijklmnopqrrqponmlkjihgfedcba"),                       // slow pulse
    (12, "mmnnmmnnnmmnn"),                                              // underwater
    (63, "a"),                                                          // testing / off
];

impl LightstyleTable {
    /// All styles full ON (`"z"`) — closest to previous “sum all styles” look.
    pub fn all_on() -> Self {
        Self {
            patterns: vec!["z".to_string(); MAX_LIGHTSTYLES],
            scales: vec![1.0; MAX_LIGHTSTYLES],
        }
    }

    /// GoldSrc defaults: styles 0–12 animated; 32–62 ON (`"z"`) until light entities
    /// drive them (we don't parse switchable yet); 63 off.
    pub fn hl_defaults() -> Self {
        let mut patterns = vec!["a".to_string(); MAX_LIGHTSTYLES];
        // Switchable light slots: default ON so maps aren't full of dark blotches.
        for i in 32..63 {
            patterns[i] = "z".to_string();
        }
        for &(i, pat) in HL_STD_PATTERNS {
            patterns[i] = pat.to_string();
        }
        let mut scales = vec![0.0; MAX_LIGHTSTYLES];
        for i in 0..MAX_LIGHTSTYLES {
            scales[i] = eval_pattern(&patterns[i], 0.0);
        }
        Self { patterns, scales }
    }

    /// Style 0 medium (`m`), rest off — closer to cold HL start.
    pub fn hl_cold() -> Self {
        let mut patterns = vec!["a".to_string(); MAX_LIGHTSTYLES];
        patterns[0] = "m".to_string();
        let mut scales = vec![0.0; MAX_LIGHTSTYLES];
        scales[0] = eval_char(b'm');
        Self { patterns, scales }
    }

    pub fn set_pattern(&mut self, style: usize, pattern: impl Into<String>) {
        if style >= MAX_LIGHTSTYLES {
            return;
        }
        if self.patterns.len() < MAX_LIGHTSTYLES {
            self.patterns.resize(MAX_LIGHTSTYLES, "a".into());
        }
        if self.scales.len() < MAX_LIGHTSTYLES {
            self.scales.resize(MAX_LIGHTSTYLES, 0.0);
        }
        self.patterns[style] = pattern.into();
    }

    /// Advance animations from `time_secs` (typically host clock).
    pub fn tick(&mut self, time_secs: f32) {
        if self.patterns.len() < MAX_LIGHTSTYLES {
            self.patterns.resize(MAX_LIGHTSTYLES, "a".into());
        }
        if self.scales.len() < MAX_LIGHTSTYLES {
            self.scales.resize(MAX_LIGHTSTYLES, 0.0);
        }
        for i in 0..MAX_LIGHTSTYLES {
            self.scales[i] = eval_pattern(&self.patterns[i], time_secs);
        }
    }

    pub fn scales_array(&self) -> [f32; MAX_LIGHTSTYLES] {
        let mut out = [0.0f32; MAX_LIGHTSTYLES];
        for (i, s) in self.scales.iter().take(MAX_LIGHTSTYLES).enumerate() {
            out[i] = *s;
        }
        out
    }
}

fn eval_pattern(pat: &str, time_secs: f32) -> f32 {
    let b = pat.as_bytes();
    if b.is_empty() {
        return 0.0;
    }
    let idx = ((time_secs * 10.0).floor() as usize) % b.len();
    eval_char(b[idx])
}

/// `'a'` → 0.0, `'z'` → 1.0 (HL-style).
fn eval_char(c: u8) -> f32 {
    if c < b'a' {
        return 0.0;
    }
    ((c - b'a') as f32 / 25.0).clamp(0.0, 1.0)
}
