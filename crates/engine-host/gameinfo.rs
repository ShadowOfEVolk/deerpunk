//! Xash-style `gameinfo.txt` key-value descriptor.

use std::collections::HashMap;
use std::path::Path;

/// Parsed game descriptor. `gamefolder` is always the gamedir directory name
/// (FWGS ignores the `gamedir` key for that purpose).
#[derive(Debug, Clone)]
pub struct GameInfo {
    pub gamefolder: String,
    pub basedir: String,
    pub title: String,
    /// Relative to gamedir (e.g. `game.ico`).
    pub icon: String,
    pub startmap: String,
    pub trainmap: String,
    pub gamedll: String,
    pub version: String,
    pub extras: HashMap<String, String>,
}

impl GameInfo {
    pub fn from_gamedir(gamedir: &Path) -> Self {
        let gamefolder = gamedir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("game")
            .to_string();
        let mut info = Self::defaults(&gamefolder);

        let gi = gamedir.join("gameinfo.txt");
        if !gi.is_file() {
            log::warn!(
                "[gameinfo] missing {} — defaults gamefolder=\"{}\"",
                gi.display(),
                info.gamefolder
            );
            return info;
        }
        match std::fs::read_to_string(&gi) {
            Ok(text) => {
                apply_kv(&mut info, &parse_kv(&text));
                log::info!(
                    "[gameinfo] {} title=\"{}\" icon=\"{}\" basedir=\"{}\"",
                    gi.display(),
                    info.title,
                    info.icon,
                    info.basedir
                );
            }
            Err(e) => log::warn!("[gameinfo] read {}: {e}", gi.display()),
        }
        info
    }

    /// Window title: `title`, or `gamefolder` if empty.
    pub fn window_title(&self) -> &str {
        if self.title.is_empty() {
            &self.gamefolder
        } else {
            &self.title
        }
    }

    /// Absolute icon path under `gamedir`, if the file exists.
    pub fn icon_path(&self, gamedir: &Path) -> Option<std::path::PathBuf> {
        let rel = if self.icon.is_empty() {
            "game.ico"
        } else {
            self.icon.as_str()
        };
        let path = gamedir.join(rel);
        if path.is_file() {
            Some(path)
        } else {
            None
        }
    }

    fn defaults(gamefolder: &str) -> Self {
        Self {
            gamefolder: gamefolder.to_string(),
            basedir: gamefolder.to_string(),
            title: gamefolder.to_string(),
            icon: "game.ico".into(),
            startmap: String::new(),
            trainmap: String::new(),
            gamedll: String::new(),
            version: String::new(),
            extras: HashMap::new(),
        }
    }
}

fn apply_kv(info: &mut GameInfo, kv: &HashMap<String, String>) {
    // Folder name wins for gamefolder (Xash FWGS).
    let folder = info.gamefolder.clone();
    for (k, v) in kv {
        match k.as_str() {
            "basedir" => info.basedir = v.clone(),
            "title" => info.title = v.clone(),
            "icon" => info.icon = v.clone(),
            "startmap" => info.startmap = v.clone(),
            "trainmap" => info.trainmap = v.clone(),
            "gamedll" => info.gamedll = v.clone(),
            "version" => info.version = v.clone(),
            "gamedir" => { /* ignored — directory name */ }
            _ => {
                info.extras.insert(k.clone(), v.clone());
            }
        }
    }
    info.gamefolder = folder;
}

/// Parse Xash gameinfo lines: `key value` / `key "value"`, `//` comments.
pub fn parse_kv(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let tokens = tokenize(line);
        if tokens.len() < 2 {
            continue;
        }
        out.insert(tokens[0].to_ascii_lowercase(), tokens[1].clone());
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let c = bytes[i] as char;
        if c == '"' {
            in_quotes = !in_quotes;
            i += 1;
            continue;
        }
        if !in_quotes && c == '/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    tokens.push(std::mem::take(&mut current));
                    in_quotes = false;
                } else {
                    in_quotes = true;
                }
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            '\\' if in_quotes => {
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() || in_quotes {
        tokens.push(current);
    }
    tokens
}

deerpunk_engine_common::cvar_def!(
    "gamedir",
    "examples/deathmatch/gamedir",
    "Game directory path"
);
deerpunk_engine_common::cvar_def!(
    "gamename",
    "",
    "Game folder name (from gamedir path / gameinfo). Used for {gamename}.rc"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_comments() {
        let kv = parse_kv(
            r#"
// generated
basedir		"valve"
title		"Dark Demo" // trailing
version		1
gamedir		"ignored"
"#,
        );
        assert_eq!(kv.get("basedir").map(String::as_str), Some("valve"));
        assert_eq!(kv.get("title").map(String::as_str), Some("Dark Demo"));
        assert_eq!(kv.get("version").map(String::as_str), Some("1"));
        assert_eq!(kv.get("gamedir").map(String::as_str), Some("ignored"));
    }

    #[test]
    fn gamefolder_from_path_not_file() {
        let dir = std::env::temp_dir().join("gi_test_darkdemo");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("gameinfo.txt"),
            "gamedir \"other\"\ntitle \"T\"\nbasedir \"base\"\n",
        )
        .unwrap();
        let info = GameInfo::from_gamedir(&dir);
        assert_eq!(info.gamefolder, "gi_test_darkdemo");
        assert_eq!(info.title, "T");
        assert_eq!(info.basedir, "base");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
