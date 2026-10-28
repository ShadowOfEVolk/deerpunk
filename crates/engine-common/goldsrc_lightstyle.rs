//! GoldSrc-style lightstyle patterns (host client path; not an ECS component).

/// Number of style slots (HL / Quake).
pub const MAX_LIGHTSTYLES: usize = 64;

/// Runtime lightstyle table — patterns + evaluated scales.
#[derive(Clone, Debug)]
pub struct LightstyleTable {
    pub patterns: Vec<String>,
    pub scales: Vec<f32>,
}

impl Default for LightstyleTable {
    fn default() -> Self {
        Self::hl_defaults()
    }
}

const HL_STD_PATTERNS: &[(usize, &str)] = &[
    (0, "m"),
    (1, "mmnmmommommnonmmonqnmmo"),
    (2, "abcdefghijklmnopqrstuvwxyzyxwvutsrqponmlkjihgfedcba"),
    (3, "mmmmmaaaaammmmmaaaaaabcdefgabcdefg"),
    (4, "mamamamamama"),
    (5, "jklmnopqrstuvwxyzyxwvutsrqponmlkj"),
    (6, "nmonqnmomnmomomno"),
    (7, "mmmaaaabcdefgmmmmaaaammmaamm"),
    (8, "mmmaaammmaaammmabcdefaaaammmmabcdefmmmaaaa"),
    (9, "aaaaaaaazzzzzzzz"),
    (10, "mmamammmmammamamaaamammma"),
    (11, "abcdefghijklmnopqrrqponmlkjihgfedcba"),
    (12, "mmnnmmnnnmmnn"),
    (63, "a"),
];

impl LightstyleTable {
    pub fn hl_defaults() -> Self {
        let mut patterns = vec!["a".to_string(); MAX_LIGHTSTYLES];
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

fn eval_char(c: u8) -> f32 {
    if c < b'a' {
        return 0.0;
    }
    ((c - b'a') as f32 / 25.0).clamp(0.0, 1.0)
}
