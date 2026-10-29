use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::cmd_completion::{
    filter_name_prefix, suggest_cvar_assignment, suggest_cvar_names_at_arg, split_for_completion,
    MAX_COMPLETIONS,
};
use crate::cvar::CvarSystem;
use crate::fs::FileSystem;

use deerpunk_engine_macro as methoddb;

const CMD_MAX_BUFFER: usize = 256;

pub struct CommandExecutionContext<'a> {
    pub cmd: &'a CmdSystem,
    pub args: Vec<&'a str>,
}

type CommandCallback = Box<dyn Fn(CommandExecutionContext) + Send + Sync>;

/// Brigadier-style: given the full console input line, return full-line tab candidates.
pub type CommandSuggestFn =
    Box<dyn for<'a> Fn(&str, &'a SuggestContext<'a>) -> Vec<String> + Send + Sync>;

pub struct SuggestContext<'a> {
    pub cvar: &'a CvarSystem,
    pub names: &'a [String],
}

struct CommandEntry {
    execute: CommandCallback,
    suggest: CommandSuggestFn,
}

/// Console command registration (`execute` required, `suggest` optional).
pub struct CommandRegistration {
    name: String,
    execute: Option<CommandCallback>,
    suggest: Option<CommandSuggestFn>,
}

impl CommandRegistration {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            execute: None,
            suggest: None,
        }
    }

    pub fn execute<F>(mut self, f: F) -> Self
    where
        F: Fn(CommandExecutionContext) + Send + Sync + 'static,
    {
        self.execute = Some(Box::new(f));
        self
    }

    pub fn suggest<S>(mut self, f: S) -> Self
    where
        S: for<'a> Fn(&str, &'a SuggestContext<'a>) -> Vec<String> + Send + Sync + 'static,
    {
        self.suggest = Some(Box::new(f));
        self
    }

    fn into_entry(self) -> (String, CommandEntry) {
        let name = self.name;
        let execute = self
            .execute
            .expect("CommandRegistration: call .execute() before register");
        let suggest = self
            .suggest
            .unwrap_or_else(|| Box::new(|_, _| Vec::new()));
        (name, CommandEntry { execute, suggest })
    }
}

/// Gamelib console callback (`v1:cmd:register`).
pub type CmdCallback =
    extern "C" fn(i32, *const *const u8, *const usize, *mut c_void);

struct DllCmd {
    callback: CmdCallback,
    user: *mut c_void,
}

unsafe impl Send for DllCmd {}
unsafe impl Sync for DllCmd {}

fn invoke_dll_cmd(dll: &DllCmd, args: &[&str]) {
    let argc = args.len() as i32;
    let ptrs: Vec<*const u8> = args.iter().map(|s| s.as_ptr()).collect();
    let lens: Vec<usize> = args.iter().map(|s| s.len()).collect();
    (dll.callback)(argc, ptrs.as_ptr(), lens.as_ptr(), dll.user);
}

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
    commands: HashMap<String, CommandEntry>,
    cvar: *const CvarSystem,
    fs: *const FileSystem,
    /// Process argv (`+map foo` …), consumed once by `stuffcmds`.
    cmdline: Vec<String>,
    stuffcmds_done: Cell<bool>,
    /// Set by built-in `quit` (main loop polls [`Self::quit_requested`]).
    quit_requested: AtomicBool,
    /// In-process client (console `s_play` and similar); same thread as host.
    #[cfg(any(feature = "client", feature = "full"))]
    client_engine: *mut deerpunk_engine_client::client_engine::ClientEngine,
    #[cfg(any(feature = "client", feature = "full"))]
    resources: *const deerpunk_engine_common::resource::ResourceManager,
}

unsafe impl Send for CmdSystem {}
unsafe impl Sync for CmdSystem {}

#[methoddb::namespace(
    name = "v1:cmd",
    host_field = "cmd",
    scope = ["server", "client"],
)]
impl CmdSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
            buffer: RefCell::new(CmdBuffer::new()),
            commands: HashMap::new(),
            cvar: std::ptr::null(),
            fs: std::ptr::null(),
            cmdline: Vec::new(),
            stuffcmds_done: Cell::new(false),
            quit_requested: AtomicBool::new(false),
            #[cfg(any(feature = "client", feature = "full"))]
            client_engine: std::ptr::null_mut(),
            #[cfg(any(feature = "client", feature = "full"))]
            resources: std::ptr::null(),
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn set_client_ref(&mut self, client: Option<&mut deerpunk_engine_client::client_engine::ClientEngine>) {
        self.client_engine = client.map_or(std::ptr::null_mut(), |c| c as *mut _);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    pub fn set_resources_ref(
        &mut self,
        resources: Option<&deerpunk_engine_common::resource::ResourceManager>,
    ) {
        self.resources = resources.map_or(std::ptr::null(), |r| r as *const _);
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn client_engine_mut(&self) -> Option<&mut deerpunk_engine_client::client_engine::ClientEngine> {
        if self.client_engine.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.client_engine })
        }
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn resources(&self) -> Option<&deerpunk_engine_common::resource::ResourceManager> {
        if self.resources.is_null() {
            None
        } else {
            Some(unsafe { &*self.resources })
        }
    }

    pub fn request_quit(&self) {
        self.quit_requested.store(true, Ordering::SeqCst);
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_requested.load(Ordering::SeqCst)
    }

    pub fn set_refs(&mut self, cvar: &CvarSystem, fs: &FileSystem) {
        self.cvar = cvar as *const CvarSystem;
        self.fs = fs as *const FileSystem;
    }

    /// Capture process argv for later `stuffcmds` (call before startup scripts).
    pub fn set_cmdline(&mut self, args: Vec<String>) {
        self.cmdline = args;
        self.stuffcmds_done.set(false);
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
        self.register_toggle_command();
        self.register_toggle_smart_command();
        self.register_stuffcmds_command();
        self.register(
            CommandRegistration::new("quit").execute(|ctx| ctx.cmd.request_quit()),
        );
        #[cfg(any(feature = "client", feature = "full"))]
        self.register_s_play_command();
        #[cfg(any(feature = "client", feature = "full"))]
        self.register_reload_shaders_command();
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn register_reload_shaders_command(&mut self) {
        self.register(CommandRegistration::new("reload_shaders").execute(|ctx| {
            let Some(client) = ctx.cmd.client_engine_mut() else {
                log::warn!("[reload_shaders] no client");
                return;
            };
            let Some(resources) = ctx.cmd.resources() else {
                log::warn!("[reload_shaders] no resources");
                return;
            };
            match client.reload_shaders(resources) {
                Ok(()) => log::info!("[reload_shaders] OK"),
                Err(e) => log::error!("[reload_shaders] {e}"),
            }
        }));
    }

    #[cfg(any(feature = "client", feature = "full"))]
    fn register_s_play_command(&mut self) {
        self.register(CommandRegistration::new("s_play").execute(|ctx| {
            let event = ctx.args.join(" ");
            if event.is_empty() {
                log::info!("[s_play] usage: s_play <event_path> (e.g. event:/ambient/fan)");
                return;
            }
            let vol = ctx.cmd.cvar().get_f32_or("s_volume", 1.0);
            if let Some(client) = ctx.cmd.client_engine_mut() {
                client.set_audio_volume(vol);
                client.audio_play_2d(&event, vol, &[], 0);
            }
        }));
    }

    /// Quake/Xash: apply `+command args` from the process command line.
    /// Intended at the end of `.rc` / after configs; runs at most once.
    fn register_stuffcmds_command(&mut self) {
        self.register(
            CommandRegistration::new("stuffcmds").execute(|ctx| {
                ctx.cmd.exec_stuffcmds();
            }),
        );
    }

    pub fn exec_stuffcmds(&self) {
        if self.stuffcmds_done.get() {
            return;
        }
        self.stuffcmds_done.set(true);
        let cmds = collect_plus_commands(self.cmdline.iter().cloned());
        if cmds.is_empty() {
            return;
        }
        for c in &cmds {
            log::info!("[stuffcmds] {c}");
            self.add_command(c.clone());
        }
        self.process_buffer();
    }

    fn register_echo_command(&mut self) {
        self.register(
            CommandRegistration::new("echo").execute(|ctx: CommandExecutionContext<'_>| {
                if ctx.args.is_empty() {
                    println!();
                    log::info!("[echo]");
                } else {
                    let line = ctx.args.join(" ");
                    println!("{line}");
                    log::info!("[echo] {line}");
                }
            }),
        );
    }

    /// Flip a cvar between 0 and 1: `toggle <cvar>`.
    fn register_toggle_command(&mut self) {
        self.register(
            CommandRegistration::new("toggle")
                .execute(|ctx: CommandExecutionContext<'_>| {
                    let Some(name) = ctx.args.first().copied() else {
                        println!("Usage: toggle <cvar>");
                        return;
                    };
                    let name = strip_cfg_quotes(name);
                    if name.is_empty() {
                        println!("Usage: toggle <cvar>");
                        return;
                    }
                    let on = ctx.cmd.cvar().get_bool_or(name, false);
                    let next = if on { "0" } else { "1" };
                    ctx.cmd.cvar().set_or_create(name, next);
                println!("toggle {name} = {next}");
            })
                .suggest(|line, ctx| suggest_cvar_names_at_arg(line, "toggle", 0, ctx.cvar)),
        );
    }

    /// Cycle integer cvar in `[min, max]`: `toggle_smart <min> <max> <cvar>`.
    /// Out of range / missing → `min`. Otherwise `cur+1`, wrap to `min` after `max`.
    fn register_toggle_smart_command(&mut self) {
        self.register(
            CommandRegistration::new("toggle_smart").execute(|ctx: CommandExecutionContext<'_>| {
                if ctx.args.len() < 3 {
                    println!("Usage: toggle_smart <min> <max> <cvar>");
                    return;
                }
                let Ok(min) = ctx.args[0].parse::<i32>() else {
                    println!("Usage: toggle_smart <min> <max> <cvar>");
                    return;
                };
                let Ok(max) = ctx.args[1].parse::<i32>() else {
                    println!("Usage: toggle_smart <min> <max> <cvar>");
                    return;
                };
                let name = strip_cfg_quotes(ctx.args[2]);
                if name.is_empty() {
                    println!("Usage: toggle_smart <min> <max> <cvar>");
                    return;
                }
                let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
                let cur = ctx.cmd.cvar().get_f32(name).map(|v| v as i32);
                let next = match cur {
                    Some(v) if (lo..=hi).contains(&v) => {
                        if v >= hi {
                            lo
                        } else {
                            v + 1
                        }
                    }
                    _ => lo,
                };
                let next_s = next.to_string();
                ctx.cmd.cvar().set_or_create(name, &next_s);
                println!("toggle_smart {name} = {next_s} (range {lo}..{hi})");
            })
            .suggest(|line, ctx| suggest_cvar_names_at_arg(line, "toggle_smart", 2, ctx.cvar)),
        );
    }

    fn register_exec_command(&mut self) {
        self.register(
            CommandRegistration::new("exec").execute(|ctx| {
                if ctx.args.is_empty() {
                    println!("Usage: exec <filename>");
                    return;
                }
                let filename = &ctx.args[0];
                ctx.cmd.exec_file(filename);
            }),
        );
    }

    fn exec_file(&self, filename: &str) {
        println!("execing {}", filename);
        log::info!(target: "console", "execing {}", filename);
        self.exec_script_file(filename);
    }

    /// Run each non-comment line as a console command (binds, cvars, etc.).
    pub fn exec_script_file(&self, path: &str) {
        let resolved = if !self.fs.is_null() {
            self.fs().resolve_path(path)
        } else {
            Path::new(path).to_path_buf()
        };
        let content = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(err) => {
                println!("couldn't exec {}: {err}", resolved.display());
                log::warn!("[cmd] couldn't exec {}: {err}", resolved.display());
                return;
            }
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
                continue;
            }
            self.add_command(line.to_string());
        }
        self.process_buffer();
    }

    pub fn register(&mut self, reg: CommandRegistration) {
        let (name, entry) = reg.into_entry();
        self.commands.insert(name, entry);
    }

    /// Tab-completion for the in-game console (full input lines).
    pub fn autocomplete(&self, line: &str, name_pool: &[String]) -> Vec<String> {
        let (completed, partial, _ends_ws) = split_for_completion(line);
        if completed.is_empty() {
            return filter_name_prefix(name_pool, &partial, MAX_COMPLETIONS);
        }

        let ctx = SuggestContext {
            cvar: self.cvar(),
            names: name_pool,
        };

        let cmd_name = &completed[0];
        if let Some(entry) = self.commands.get(cmd_name) {
            let from_cmd = (entry.suggest)(line, &ctx);
            if !from_cmd.is_empty() {
                return from_cmd;
            }
        }

        if completed.len() == 1 && self.cvar().contains(cmd_name) {
            let from_cvar = suggest_cvar_assignment(line, self.cvar());
            if !from_cvar.is_empty() {
                return from_cvar;
            }
        }

        Vec::new()
    }

    /// Sorted console command names (for in-game console autocomplete).
    pub fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.commands.keys().cloned().collect();
        names.sort();
        names
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

    #[methoddb::bind(name = "register")]
    pub fn api_register(
        &mut self,
        name: &str,
        callback: CmdCallback,
        user: *mut c_void,
    ) -> i32 {
        let name = name.trim();
        if name.is_empty() {
            log::error!("[cmd] register: empty name");
            return -1;
        }
        let dll = DllCmd { callback, user };
        let name_owned = name.to_string();
        log::info!("[cmd] registered gamelib command '{name_owned}'");
        self.register(
            CommandRegistration::new(name_owned).execute(move |ctx: CommandExecutionContext<'_>| {
                invoke_dll_cmd(&dll, &ctx.args);
            }),
        );
        0
    }

    #[methoddb::bind(name = "execute")]
    pub fn execute_command(&self, command_line: &str) {
        let tokens = self.tokenize(command_line);
        if tokens.is_empty() {
            return;
        }
        let command_name = tokens[0].as_str();
        let args: Vec<&str> = tokens[1..].iter().map(|s| s.as_str()).collect();

        if let Some(entry) = self.commands.get(command_name) {
            (entry.execute)(CommandExecutionContext {
                cmd: self,
                args: args.to_vec(),
            });
        } else if !args.is_empty() {
            // GoldSrc / Xash: `name value` sets silently (no echo).
            self.cvar().set_or_create(command_name, &args.join(" "));
        } else if args.is_empty() {
            if !self.cvar().print_query(command_name) {
                log::info!("Unknown command: {}", command_name);
            }
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

/// Collect Quake-style `+cmd args` from argv.
/// A `+`/`-` token (not `+1` / `-1` numeric) starts/ends a statement.
pub fn collect_plus_commands<I: Iterator<Item = String>>(args: I) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending: Option<Vec<String>> = None;
    for arg in args {
        if is_cmdline_switch(&arg, '+') {
            if let Some(parts) = pending.take() {
                out.push(parts.join(" "));
            }
            pending = Some(vec![arg[1..].to_string()]);
        } else if is_cmdline_switch(&arg, '-') {
            if let Some(parts) = pending.take() {
                out.push(parts.join(" "));
            }
        } else if let Some(parts) = pending.as_mut() {
            parts.push(arg);
        }
    }
    if let Some(parts) = pending.take() {
        out.push(parts.join(" "));
    }
    out
}

fn is_cmdline_switch(arg: &str, prefix: char) -> bool {
    let mut chars = arg.chars();
    if chars.next() != Some(prefix) {
        return false;
    }
    // Quake: `+1` / `-3` are not command switches.
    !matches!(chars.next(), Some('0'..='9'))
}

impl deerpunk_engine_common::console_cmd::ConsoleCmdSink for CmdSystem {
    fn cmd_names(&self) -> Vec<String> {
        self.command_names()
    }
    fn cmd_add(&self, command: String) {
        self.add_command(command);
    }
    fn cmd_process(&self) {
        self.process_buffer();
    }
    fn cmd_autocomplete(&self, line: &str, name_pool: &[String]) -> Vec<String> {
        self.autocomplete(line, name_pool)
    }
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

    #[test]
    fn dash_terminates_plus_args() {
        assert_eq!(
            collect_plus_commands(
                args(&["+map", "school", "-dev", "3", "+disconnect"]).into_iter()
            ),
            vec!["map school".to_string(), "disconnect".to_string()]
        );
    }

    #[test]
    fn numeric_plus_is_not_a_command() {
        assert_eq!(
            collect_plus_commands(args(&["+map", "c1a0", "+1", "ignored"]).into_iter()),
            vec!["map c1a0 +1 ignored".to_string()]
        );
    }
}
