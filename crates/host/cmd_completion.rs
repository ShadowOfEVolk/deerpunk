//! Console tab-completion helpers (Brigadier-style `suggest(line)` per command).

use crate::cvar::CvarSystem;

pub const MAX_COMPLETIONS: usize = 12;

/// Split input for completion: tokens before the edited word, partial token, trailing space.
pub fn split_for_completion(line: &str) -> (Vec<String>, String, bool) {
    let ends_ws = line.ends_with(' ') || line.ends_with('\t');
    let trimmed = line.trim_end_matches([' ', '\t']);
    let mut parts: Vec<String> = trimmed
        .split_whitespace()
        .map(String::from)
        .collect();
    if ends_ws {
        return (parts, String::new(), true);
    }
    if parts.is_empty() {
        return (Vec::new(), line.trim().to_string(), false);
    }
    let partial = parts.pop().unwrap();
    (parts, partial, false)
}

/// Index of the command argument currently being edited (`0` = first arg after command name).
pub fn completing_arg_index(completed: &[String], _partial: &str, ends_ws: bool) -> Option<usize> {
    if completed.is_empty() {
        return None;
    }
    let args_before = completed.len() - 1;
    if ends_ws {
        Some(args_before)
    } else {
        Some(args_before)
    }
}

pub fn build_completed_line(completed: &[String], new_token: &str) -> String {
    let mut parts = completed.to_vec();
    parts.push(new_token.to_string());
    parts.join(" ")
}

pub fn filter_name_prefix(pool: &[String], partial: &str, max: usize) -> Vec<String> {
    let p = partial.to_ascii_lowercase();
    if p.is_empty() {
        return Vec::new();
    }
    pool.iter()
        .filter(|n| n.to_ascii_lowercase().starts_with(&p))
        .take(max)
        .cloned()
        .collect()
}

/// Complete one argument from a fixed name pool (full input lines).
pub fn suggest_command_arg_names(
    line: &str,
    cmd: &str,
    arg_index: usize,
    pool: &[String],
) -> Vec<String> {
    let (completed, partial, ends_ws) = split_for_completion(line);
    if completed.first().map(|s| s.as_str()) != Some(cmd) {
        return Vec::new();
    }
    let cur_arg = completing_arg_index(&completed, &partial, ends_ws);
    if cur_arg != Some(arg_index) {
        return Vec::new();
    }
    let p = partial.to_ascii_lowercase();
    let mut out = Vec::new();
    for name in pool {
        if !p.is_empty() && !name.to_ascii_lowercase().starts_with(&p) {
            continue;
        }
        out.push(build_completed_line(&completed, name));
        if out.len() >= MAX_COMPLETIONS {
            break;
        }
    }
    out
}

pub fn suggest_cvar_names_at_arg(
    line: &str,
    cmd: &str,
    arg_index: usize,
    cvar: &CvarSystem,
) -> Vec<String> {
    let names: Vec<String> = cvar.dump().into_iter().map(|(n, _)| n).collect();
    suggest_command_arg_names(line, cmd, arg_index, &names)
}

/// `cvar [value]` — suggest bool-ish values when setting a known cvar.
pub fn suggest_cvar_assignment(line: &str, cvar: &CvarSystem) -> Vec<String> {
    let (completed, partial, ends_ws) = split_for_completion(line);
    if completed.len() != 1 {
        return Vec::new();
    }
    let name = &completed[0];
    if !cvar.contains(name) {
        return Vec::new();
    }
    // Value token: after `name ` or while editing the second token.
    if !ends_ws && partial.is_empty() {
        return Vec::new();
    }
    const BOOLISH: [&str; 6] = ["0", "1", "true", "false", "on", "off"];
    let p = partial.to_ascii_lowercase();
    let mut out = Vec::new();
    for v in BOOLISH {
        if !p.is_empty() && !v.starts_with(p.as_str()) {
            continue;
        }
        out.push(build_completed_line(&completed, v));
        if out.len() >= MAX_COMPLETIONS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_partial_arg() {
        let (c, p, ws) = split_for_completion("toggle cl_con");
        assert_eq!(c, vec!["toggle".to_string()]);
        assert_eq!(p, "cl_con");
        assert!(!ws);
    }

    #[test]
    fn split_after_space() {
        let (c, p, ws) = split_for_completion("toggle ");
        assert_eq!(c, vec!["toggle".to_string()]);
        assert!(p.is_empty());
        assert!(ws);
    }
}
