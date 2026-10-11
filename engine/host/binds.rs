//! Host keybinds: `bind <key> <command>`, `unbind <key>`.
//!
//! Fires on key press edge while the in-game console is closed.
//! Keys are USB HID scancode names (layout-independent), Quake-style.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use deerpunk_engine_common::input::{Input, VirtualKey};

use crate::cmd::{CmdSystem, CommandExecutionContext};

/// Process-wide bind table (host console owns it).
pub fn global() -> &'static Binds {
    static BINDS: OnceLock<Binds> = OnceLock::new();
    BINDS.get_or_init(Binds::new)
}

/// Shared bind table (host-owned; console commands mutate it).
pub struct Binds {
    map: Mutex<HashMap<VirtualKey, String>>,
    /// Previous-frame down state for bound keys (edge detect).
    prev_down: Mutex<HashMap<VirtualKey, bool>>,
}

impl Default for Binds {
    fn default() -> Self {
        Self::new()
    }
}

impl Binds {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            prev_down: Mutex::new(HashMap::new()),
        }
    }

    pub fn set(&self, key: VirtualKey, command: String) {
        self.map.lock().unwrap().insert(key, command);
    }

    pub fn remove(&self, key: VirtualKey) -> bool {
        self.map.lock().unwrap().remove(&key).is_some()
    }

    pub fn get(&self, key: VirtualKey) -> Option<String> {
        self.map.lock().unwrap().get(&key).cloned()
    }

    pub fn list(&self) -> Vec<(VirtualKey, String)> {
        let mut v: Vec<_> = self
            .map
            .lock()
            .unwrap()
            .iter()
            .map(|(k, c)| (*k, c.clone()))
            .collect();
        v.sort_by_key(|(k, _)| *k);
        v
    }

    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
        self.prev_down.lock().unwrap().clear();
    }

    /// Register `bind` / `unbind` / `unbindall` on the host console.
    pub fn register_commands(cmd: &mut CmdSystem) {
        cmd.register_command("bind", |ctx| bind_cmd(global(), ctx));
        cmd.register_command("unbind", |ctx| unbind_cmd(global(), ctx));
        cmd.register_command("unbindall", |_ctx| {
            global().clear();
            println!("unbindall");
        });
    }

    /// Press-edge: enqueue bound commands. Skip while console is open.
    pub fn tick(&self, input: &Input, cmd: &CmdSystem, console_open: bool) {
        if console_open {
            // Don't leave stale edges after console ate keys.
            if let Ok(mut prev) = self.prev_down.lock() {
                prev.clear();
            }
            return;
        }

        let map = self.map.lock().unwrap().clone();
        let mut prev = self.prev_down.lock().unwrap();
        let mut fired = Vec::new();
        for (key, command) in &map {
            let down = input.is_down(*key);
            let was = prev.get(key).copied().unwrap_or(false);
            if down && !was && !command.is_empty() {
                fired.push(command.clone());
            }
            prev.insert(*key, down);
        }
        prev.retain(|k, _| map.contains_key(k));
        drop(prev);
        for command in fired {
            cmd.add_command(command);
        }
        cmd.process_buffer();
    }
}

fn bind_cmd(binds: &Binds, ctx: CommandExecutionContext<'_>) {
    if ctx.args.is_empty() {
        let list = binds.list();
        if list.is_empty() {
            println!("bind: (none)");
            return;
        }
        for (key, command) in list {
            println!("\"{}\" = \"{}\"", key_name(key), command);
        }
        return;
    }

    let key_tok = ctx.args[0];
    let Some(key) = parse_key(key_tok) else {
        println!("bind: unknown key '{key_tok}'");
        return;
    };

    if ctx.args.len() == 1 {
        match binds.get(key) {
            Some(cmd) => println!("\"{}\" = \"{}\"", key_name(key), cmd),
            None => println!("\"{}\" is not bound", key_name(key)),
        }
        return;
    }

    let command = ctx.args[1..].join(" ");
    binds.set(key, command.clone());
    println!("bind \"{}\" \"{}\"", key_name(key), command);
}

fn unbind_cmd(binds: &Binds, ctx: CommandExecutionContext<'_>) {
    let Some(key_tok) = ctx.args.first().copied() else {
        println!("Usage: unbind <key>");
        return;
    };
    let Some(key) = parse_key(key_tok) else {
        println!("unbind: unknown key '{key_tok}'");
        return;
    };
    if binds.remove(key) {
        println!("unbind \"{}\"", key_name(key));
    } else {
        println!("\"{}\" is not bound", key_name(key));
    }
}

/// Parse Quake-ish key token → USB HID scancode.
pub fn parse_key(name: &str) -> Option<VirtualKey> {
    let n = name.trim().to_ascii_lowercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        return match c {
            b'a'..=b'z' => Some(4 + (c - b'a') as VirtualKey),
            b'1'..=b'9' => Some(30 + (c - b'1') as VirtualKey),
            b'0' => Some(39),
            _ => None,
        };
    }
    if let Some(rest) = n.strip_prefix('f') {
        if let Ok(i) = rest.parse::<u16>() {
            if (1..=12).contains(&i) {
                return Some(57 + i); // F1=58
            }
        }
    }
    match n.as_str() {
        "space" => Some(44),
        "tab" => Some(43),
        "escape" | "esc" => Some(41),
        "enter" | "return" => Some(40),
        "backspace" | "back" => Some(42),
        "capslock" | "caps" => Some(57),
        "uparrow" | "up" => Some(82),
        "downarrow" | "down" => Some(81),
        "leftarrow" | "left" => Some(80),
        "rightarrow" | "right" => Some(79),
        "lctrl" | "ctrl" => Some(224),
        "lshift" | "shift" => Some(225),
        "lalt" | "alt" => Some(226),
        "rctrl" => Some(228),
        "rshift" => Some(229),
        "ralt" => Some(230),
        "grave" | "tilde" | "`" => Some(53),
        "minus" | "-" => Some(45),
        "equals" | "=" => Some(46),
        "lbracket" | "[" => Some(47),
        "rbracket" | "]" => Some(48),
        "backslash" | "\\" => Some(49),
        "semicolon" | ";" => Some(51),
        "apostrophe" | "'" => Some(52),
        "comma" | "," => Some(54),
        "period" | "." => Some(55),
        "slash" | "/" => Some(56),
        _ => None,
    }
}

pub fn key_name(key: VirtualKey) -> String {
    match key {
        4..=29 => ((b'a' + (key - 4) as u8) as char).to_string(),
        30..=38 => ((b'1' + (key - 30) as u8) as char).to_string(),
        39 => "0".into(),
        40 => "enter".into(),
        41 => "escape".into(),
        42 => "backspace".into(),
        43 => "tab".into(),
        44 => "space".into(),
        45 => "minus".into(),
        46 => "equals".into(),
        47 => "lbracket".into(),
        48 => "rbracket".into(),
        49 => "backslash".into(),
        51 => "semicolon".into(),
        52 => "apostrophe".into(),
        53 => "grave".into(),
        54 => "comma".into(),
        55 => "period".into(),
        56 => "slash".into(),
        57 => "capslock".into(),
        58..=69 => format!("f{}", key - 57),
        79 => "right".into(),
        80 => "left".into(),
        81 => "down".into(),
        82 => "up".into(),
        224 => "lctrl".into(),
        225 => "lshift".into(),
        226 => "lalt".into(),
        228 => "rctrl".into(),
        229 => "rshift".into(),
        230 => "ralt".into(),
        other => format!("key{other}"),
    }
}
