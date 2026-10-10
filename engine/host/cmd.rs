use std::{cell::RefCell, collections::HashMap};
use std::path::Path;

use crate::cvar::CvarSystem;
use crate::fs::FileSystem;

const CMD_MAX_BUFFER: usize = 256;

pub struct CommandExecutionContext<'a> {
    pub cmd: &'a CmdSystem,
    pub args: Vec<&'a str>,
}

type CommandCallback = Box<dyn Fn(CommandExecutionContext) + Send + Sync>;

pub struct CmdBuffer {
    commands: [Option<String>; CMD_MAX_BUFFER],
    write_pos: usize,
    read_pos: usize,
    count: usize,
}

impl CmdBuffer {
    pub fn new() -> Self {
        Self {
            commands: [(); CMD_MAX_BUFFER].map(|_| None),
            write_pos: 0,
            read_pos: 0,
            count: 0,
        }
    }
}

pub struct CmdSystem {
    initialized: bool,
    buffer: RefCell<CmdBuffer>,
    commands: HashMap<String, CommandCallback>,
    cvar: *const CvarSystem,
    fs: *const FileSystem,
}

unsafe impl Send for CmdSystem {}
unsafe impl Sync for CmdSystem {}

impl CmdSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
            buffer: RefCell::new(CmdBuffer::new()),
            commands: HashMap::new(),
            cvar: std::ptr::null(),
            fs: std::ptr::null(),
        }
    }

    pub fn set_refs(&mut self, cvar: &CvarSystem, fs: &FileSystem) {
        self.cvar = cvar as *const CvarSystem;
        self.fs = fs as *const FileSystem;
    }

    fn cvar(&self) -> &CvarSystem {
        unsafe { &*self.cvar }
    }

    fn fs(&self) -> &FileSystem {
        unsafe { &*self.fs }
    }

    pub fn initialize(&mut self) {
        self.initialized = true;
        self.register_exec_command();
        self.register_echo_command();
    }

    fn register_echo_command(&mut self) {
        self.register_command(
            "echo",
            Box::new(move |ctx: CommandExecutionContext<'_>| {
                if ctx.args.is_empty() {
                    println!();
                } else {
                    println!("{}", ctx.args.join(" "));
                }
            }),
        );
    }

    fn register_exec_command(&mut self) {
        let exec_handler: CommandCallback = Box::new(move |ctx| {
            if ctx.args.is_empty() {
                println!("Usage: exec <filename>");
                return;
            }
            let filename = &ctx.args[0];
            ctx.cmd.exec_file(filename);
        });
        self.commands.insert("exec".to_string(), exec_handler);
    }

    fn exec_file(&self, filename: &str) {
        println!("Executing file: {}", filename);
        let Ok(mut iterator) = self.fs().stream_file(filename) else {
            println!("Failed to open file: {}", filename);
            return;
        };
        for line in iterator.lines() {
            let line = line.unwrap();
            self.add_command(line);
        }
        self.process_buffer();
    }

    pub fn register_command<F>(&mut self, name: &str, callback: F)
    where
        F: Fn(CommandExecutionContext) + Send + Sync + 'static,
    {
        self.commands.insert(name.to_string(), Box::new(callback));
    }

    pub fn shutdown(&mut self) {
        self.initialized = false;
        self.buffer.replace(CmdBuffer::new());
    }

    pub fn frame(&mut self) {
        if !self.initialized {
            return;
        }
        self.process_buffer();
    }

    pub fn process_buffer(&self) {
        let commands_to_execute = {
            let mut buffer = self.buffer.borrow_mut();
            let mut commands = Vec::new();
            let initial_count = buffer.count;
            for _ in 0..initial_count {
                let read_pos = buffer.read_pos;
                if let Some(command_line) = buffer.commands[read_pos].take() {
                    buffer.read_pos = (read_pos + 1) % CMD_MAX_BUFFER;
                    buffer.count -= 1;
                    commands.push(command_line);
                } else {
                    buffer.read_pos = (read_pos + 1) % CMD_MAX_BUFFER;
                    buffer.count = buffer.count.saturating_sub(1);
                }
            }
            commands
        };
        for command in commands_to_execute {
            self.execute_command(&command);
        }
    }

    pub fn add_command(&self, command: String) {
        let mut buffer = self.buffer.borrow_mut();
        let write_pos = buffer.write_pos;
        if buffer.count >= CMD_MAX_BUFFER {
            buffer.read_pos = (buffer.read_pos + 1) % CMD_MAX_BUFFER;
            buffer.count -= 1;
        }
        buffer.commands[write_pos] = Some(command);
        buffer.write_pos = (buffer.write_pos + 1) % CMD_MAX_BUFFER;
        buffer.count += 1;
    }

    pub fn execute_command(&self, command_line: &str) {
        let tokens = self.tokenize(command_line);
        if tokens.is_empty() {
            return;
        }
        let command_name = tokens[0].as_str();
        let args: Vec<&str> = tokens[1..].iter().map(|s| s.as_str()).collect();

        if let Some(callback) = self.commands.get(command_name) {
            println!("Executing command: {}", command_name);
            callback(CommandExecutionContext {
                cmd: self,
                args: args.to_vec(),
            });
        } else if !args.is_empty() {
            self.cvar().set_or_create(command_name, &args.join(" "));
            println!("Setting cvar: {} = {}", command_name, args.join(" "));
        } else {
            println!("Unknown command: {}", command_name);
        }
    }

    pub fn exec_config_file(&self, path: &str) {
        let resolved = if !self.fs.is_null() {
            self.fs().resolve_path(path)
        } else {
            Path::new(path).to_path_buf()
        };
        let content = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("[cfg] can't read {}: {err}", resolved.display());
                return;
            }
        };
        eprintln!("[cfg] executing {}", resolved.display());
        let mut executed = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
                continue;
            }
            if let Some((name, value)) = line.split_once(char::is_whitespace) {
                let name = name.trim();
                let value = strip_cfg_quotes(value.trim());
                if !self.cvar().set(name, value) {
                    self.cvar().set_or_create(name, value);
                }
                executed += 1;
            } else {
                eprintln!("[cfg] {path}: skipping malformed line: {line}");
            }
        }
        if executed > 0 {
            println!("[cfg] {path}: applied {executed} cvars");
        }
    }

    fn tokenize(&self, line: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    if in_quotes {
                        if !current.is_empty() {
                            tokens.push(current.clone());
                            current.clear();
                        }
                        in_quotes = false;
                    } else {
                        in_quotes = true;
                    }
                }
                ' ' | '\t' if !in_quotes => {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                }
                _ => {
                    current.push(ch);
                }
            }
        }
        if !current.is_empty() || in_quotes {
            tokens.push(current);
        }
        tokens
    }
}

impl Default for CmdSystem {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip matching surrounding quotes from a cfg value (`"0.0.0.0"` → `0.0.0.0`).
fn strip_cfg_quotes(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 {
        let bytes = v.as_bytes();
        if (bytes[0] == b'"' && bytes[v.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[v.len() - 1] == b'\'')
        {
            return &v[1..v.len() - 1];
        }
    }
    v
}

pub fn collect_plus_commands<I: Iterator<Item = String>>(args: I) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending: Option<Vec<String>> = None;
    for arg in args {
        if let Some(cmd) = arg.strip_prefix('+') {
            if let Some(parts) = pending.take() {
                out.push(parts.join(" "));
            }
            pending = Some(vec![cmd.to_string()]);
        } else if let Some(parts) = pending.as_mut() {
            parts.push(arg);
        }
    }
    if let Some(parts) = pending.take() {
        out.push(parts.join(" "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::collect_plus_commands;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn collects_plus_commands() {
        assert_eq!(
            collect_plus_commands(args(&["+ai_enabled", "0", "+ecs_showsystems", "1"]).into_iter()),
            vec!["ai_enabled 0".to_string(), "ecs_showsystems 1".to_string()]
        );
    }

    #[test]
    fn collects_single_arg_command() {
        assert_eq!(
            collect_plus_commands(args(&["+ai_enabled", "0"]).into_iter()),
            vec!["ai_enabled 0".to_string()]
        );
    }

    #[test]
    fn ignores_args_without_plus_prefix() {
        assert_eq!(
            collect_plus_commands(
                args(&["--features", "window", "+toggle", "show_hud"]).into_iter()
            ),
            vec!["toggle show_hud".to_string()]
        );
    }
}
