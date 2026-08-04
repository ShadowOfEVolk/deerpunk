use std::{cell::RefCell, collections::HashMap};

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
