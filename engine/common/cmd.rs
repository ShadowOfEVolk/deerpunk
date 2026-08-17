use std::{cell::RefCell, collections::HashMap};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;

use crate::cvar::cvar;
use crate::fs::fs;

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
}

impl CmdSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
            buffer: RefCell::new(CmdBuffer::new()),
            commands: HashMap::new(),
        }
    }

    pub fn initialize(&mut self) {
        self.initialized = true;

        // Register built-in commands
        self.register_exec_command();
        self.register_echo_command();

        // game.rc will be executed from core after fs is initialized
        self.add_command("exec game.rc".to_string());
    }

    fn register_echo_command(&mut self) {
        self.register_command(
            "echo",
            Box::new(move |ctx: CommandExecutionContext<'_>| {
                if ctx.args.is_empty() {
                    println!();
                } else {
                    // Join all arguments with spaces
                    println!("{}", ctx.args.join(" "));
                }
            }),
        );
    }

    fn register_exec_command(&mut self) {
        // exec command will be registered with a callback that uses the load_file function
        // We'll set it up to use a function pointer that can access fs
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
        let Ok(mut iterator) = fs().stream_file(filename) else {
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

        // Read console input (non-blocking)
        self.read_console_input();

        // Process commands from buffer
        self.process_buffer();
    }

    /// Process all commands in buffer immediately
    pub fn process_buffer(&self) {
        // TODO: подумать над тем, как не копировать данные из буфера
        // И в то же время не попасться на уловки Rust.
        // Фактически нам нужно заборровить только часть буффера и оставить возможность его модифицировать
        // Со стороны execute_command
        let commands_to_execute = {
            let mut buffer = self.buffer.borrow_mut();
            let mut commands = Vec::new();
            let initial_count = buffer.count;

            // Extract all available commands at this moment
            for _ in 0..initial_count {
                let read_pos = buffer.read_pos;

                if let Some(command_line) = buffer.commands[read_pos].take() {
                    buffer.read_pos = (read_pos + 1) % CMD_MAX_BUFFER;
                    buffer.count -= 1;
                    commands.push(command_line);
                } else {
                    // Should not happen, but handle gracefully
                    buffer.read_pos = (read_pos + 1) % CMD_MAX_BUFFER;
                    buffer.count = buffer.count.saturating_sub(1);
                }
            }

            commands
        };

        // Execute all collected commands (buffer is now free for new commands)
        for command in commands_to_execute {
            self.execute_command(&command);
        }
    }

    fn read_console_input(&mut self) {
        // Note: read_line is blocking, so this will block until input is available
        // For non-blocking input, we'd need async I/O or a separate thread
        // For now, this is fine for a simple console

        // Try to read from stdin (this will block if no input available)
        // In a real implementation, this should be in a separate thread
        // For now, we'll skip stdin reading in frame() to avoid blocking
        // Users can add commands programmatically via add_command()
    }

    pub fn add_command(&self, command: String) {
        let mut buffer = self.buffer.borrow_mut();
        let write_pos = buffer.write_pos;

        // Check if buffer is full
        if buffer.count >= CMD_MAX_BUFFER {
            // Buffer full, skip oldest command
            buffer.read_pos = (buffer.read_pos + 1) % CMD_MAX_BUFFER;
            buffer.count -= 1;
        }

        // Add command to buffer
        buffer.commands[write_pos] = Some(command);
        buffer.write_pos = (buffer.write_pos + 1) % CMD_MAX_BUFFER;
        buffer.count += 1;
    }

    pub fn execute_command(&self, command_line: &str) {
        // Tokenize command line (handles quotes like Xash3d/GoldSource)
        let tokens = self.tokenize(command_line);

        if tokens.is_empty() {
            return;
        }

        let command_name = tokens[0].as_str();
        let args: Vec<&str> = tokens[1..].iter().map(|s| s.as_str()).collect();

        // Search for registered command
        if let Some(callback) = self.commands.get(command_name) {
            println!("Executing command: {}", command_name);
            callback(CommandExecutionContext {
                cmd: self,
                args: args.to_vec(),
            });
        } else if !args.is_empty() {
            // Неизвестная команда с аргументами — попробовать как установку квара
            // (`<cvarname> <value>`), как в tanks/Quake-консоли.
            cvar().set_or_create(command_name, &args.join(" "));
            println!("Setting cvar: {} = {}", command_name, args.join(" "));
        } else {
            println!("Unknown command: {}", command_name);
        }
    }

    /// Загрузить и выполнить .cfg файл из CWD (не через gamedir).
    /// Строки `name value` применяются как квары (`set_or_create`).
    pub fn exec_config_file(&self, path: &str) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("[cfg] can't read {}: {err}", path);
                return;
            }
        };
        eprintln!("[cfg] executing {path}");
        let mut executed = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
                continue;
            }
            if let Some((name, value)) = line.split_once(char::is_whitespace) {
                let name = name.trim();
                let value = value.trim();
                if !cvar().set(name, value) {
                    // неизвестный кvar — всё равно задаём (.cfg может создавать квары)
                    cvar().set_or_create(name, value);
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

    /// Execute commands from file lines iterator
    pub fn execute_file_lines<I>(&mut self, lines: I)
    where
        I: Iterator<Item = Result<String, std::io::Error>>,
    {
        for line_result in lines {
            let line = match line_result {
                Ok(line) => line,
                Err(e) => {
                    eprintln!("Error reading config line: {}", e);
                    continue;
                }
            };

            // Lines iterator already handles comments, just trim
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Remove inline comments (if any)
            let command = if let Some(comment_pos) = trimmed.find("//") {
                &trimmed[..comment_pos]
            } else {
                trimmed
            }
            .trim();

            if !command.is_empty() {
                self.execute_command(command);
            }
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
                        // End of quoted string
                        if !current.is_empty() {
                            tokens.push(current.clone());
                            current.clear();
                        }
                        in_quotes = false;
                    } else {
                        // Start of quoted string
                        in_quotes = true;
                    }
                }
                ' ' | '\t' if !in_quotes => {
                    // Whitespace outside quotes - end of token
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

        // Add last token if any
        if !current.is_empty() || in_quotes {
            tokens.push(current);
        }

        tokens
    }
}

crate::singleton_system!(cmd, CmdSystem);

impl Default for CmdSystem {
    fn default() -> Self {
        Self::new()
    }
}

// ── TCP Command Server ──────────────────────────────────────────────

const DEFAULT_CMD_PORT: u16 = 9998;
const DEFAULT_CMD_BIND: &str = "0.0.0.0";

pub enum CmdResult {
    Handled(String),
    Pass,
}

pub type CmdHandler = Arc<dyn Fn(&str, &[&str]) -> CmdResult + Send + Sync>;

pub fn spawn_stdin(handlers: Vec<CmdHandler>) {
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    let response = handle_command(line.trim(), &handlers);
                    eprint!("[stdin] {response}");
                }
                Err(_) => break,
            }
        }
    });
}

pub struct CmdServer {
    handlers: Vec<CmdHandler>,
}

impl CmdServer {
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    pub fn add_handler(&mut self, handler: CmdHandler) {
        self.handlers.push(handler);
    }

    pub fn start(handlers: Vec<CmdHandler>) -> Self {
        let server = Self { handlers };
        server.spawn_tcp();
        server
    }

    fn spawn_tcp(&self) {
        let port = std::env::var("CMD_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_CMD_PORT);
        let bind_host = std::env::var("CMD_BIND").unwrap_or_else(|_| DEFAULT_CMD_BIND.into());
        let addr: SocketAddr = format!("{bind_host}:{port}")
            .parse()
            .unwrap_or_else(|_| {
                eprintln!("[cmd] invalid CMD_BIND/CMD_PORT, falling back to 0.0.0.0:{DEFAULT_CMD_PORT}");
                format!("0.0.0.0:{DEFAULT_CMD_PORT}").parse().unwrap()
            });

        let listener = match TcpListener::bind(addr) {
            Ok(l) => l,
            Err(err) => {
                eprintln!("[cmd] bind {addr} failed: {err}");
                return;
            }
        };

        eprintln!("[cmd] listening on {addr} — <cvar> <value> or game commands");

        let handlers = Arc::new(self.handlers.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let hs = handlers.clone();
                        thread::spawn(move || {
                            handle_client(stream, hs);
                        });
                    }
                    Err(err) => eprintln!("[cmd] accept error: {err}"),
                }
            }
        });
    }
}

fn handle_client(stream: TcpStream, handlers: Arc<Vec<CmdHandler>>) {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    if let Err(err) = stream.set_nodelay(true) {
        eprintln!("[cmd] nodelay failed for {peer}: {err}");
    }

    let reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut writer = stream;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let response = handle_command(line.trim(), &handlers);
        if let Err(err) = writer.write_all(response.as_bytes()) {
            eprintln!("[cmd] write to {peer} failed: {err}");
            break;
        }
    }
}

pub fn handle_command(line: &str, handlers: &[CmdHandler]) -> String {
    if line.is_empty() {
        return "OK\n".into();
    }
    let (cmd, rest) = match line.split_once(char::is_whitespace) {
        Some((c, r)) => (c, r.trim()),
        None => (line, ""),
    };

    if !rest.is_empty() && cvar().set(cmd, rest) {
        return format!("OK {cmd} = {rest}\n");
    }

    match cmd {
        "help" | "cvarlist" => {
            let dump = cvar().dump();
            let mut out = String::new();
            for (name, value) in &dump {
                out.push_str(&format!("{name} = {value}\n"));
            }
            out
        }
        "ping" => "OK pong\n".into(),
        "echo" => format!("{rest}\n"),
        "toggle" => {
            let name = rest.trim();
            if name.is_empty() {
                "ERR usage: toggle <cvar>\n".into()
            } else if let Some(val) = cvar().get_string(name) {
                let new = if val == "0" { "1" } else { "0" };
                cvar().set(name, new);
                format!("OK {name} = {new}\n")
            } else {
                format!("ERR unknown cvar '{name}'\n")
            }
        }
        "exec" => {
            let path = Path::new(rest.trim());
            crate::cmd::cmd().exec_config_file(rest.trim());
            format!("OK exec {}\n", path.display())
        }
        _ => {
            for handler in handlers {
                match handler(cmd, &rest.split_whitespace().collect::<Vec<_>>()) {
                    CmdResult::Handled(response) => return response,
                    CmdResult::Pass => {}
                }
            }
            format!("ERR unknown command '{cmd}'\n")
        }
    }
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

pub fn exec_plus_args(handlers: &[CmdHandler]) {
    for line in collect_plus_commands(std::env::args().skip(1)) {
        let response = handle_command(&line, handlers);
        eprintln!("[args] {response}");
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
}
