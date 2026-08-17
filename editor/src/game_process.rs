use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use deerpunk_engine_common::ipc::{IpcClient, IpcMessage};

use crate::project::{EngineLaunch, ProjectConfig};

pub struct GameProcess {
    child: Child,
    stdin_tx: Option<mpsc::Sender<String>>,
    log_rx: mpsc::Receiver<LogLine>,
    _threads: Vec<thread::JoinHandle<()>>,
    ipc_tx: Option<mpsc::Sender<IpcMessage>>,
    pub ipc_rx: mpsc::Receiver<IpcMessage>,
    shutdown: mpsc::Sender<()>,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub source: LogSource,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogSource {
    Stdout,
    Stderr,
    Editor,
}

impl LogSource {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => LogSource::Stdout,
            1 => LogSource::Stderr,
            _ => LogSource::Editor,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            LogSource::Stdout => 0,
            LogSource::Stderr => 1,
            LogSource::Editor => 2,
        }
    }
}

pub enum BuildResult {
    Success(PathBuf),
    Failed(String),
}

impl GameProcess {
    pub fn resolved_binary(project: &ProjectConfig) -> Result<PathBuf, String> {
        match &project.engine {
            EngineLaunch::Binary(path) => {
                if path.exists() {
                    Ok(path.clone())
                } else {
                    Err(format!("Engine binary not found: {}", path.display()))
                }
            }
            EngineLaunch::Cargo { bin, .. } => {
                let bin_name = if cfg!(windows) {
                    format!("{bin}.exe")
                } else {
                    bin.clone()
                };
                let path = project.root.join("target/debug").join(&bin_name);
                if path.exists() {
                    Ok(path)
                } else {
                    Err(format!(
                        "Binary not found: {} (run cargo build first)",
                        path.display()
                    ))
                }
            }
        }
    }

    pub fn build(project: &ProjectConfig, log_tx: mpsc::Sender<LogLine>) -> BuildResult {
        let (features, bin) = match &project.engine {
            EngineLaunch::Cargo { features, bin } => (features.clone(), bin.clone()),
            EngineLaunch::Binary(path) => {
                return BuildResult::Success(path.clone());
            }
        };

        let _ = log_tx.send(LogLine {
            source: LogSource::Editor,
            text: "Building project...".into(),
        });

        let mut cmd = Command::new("cargo");
        cmd.arg("build");
        if let Some(feat) = &features {
            cmd.args(["--features", feat]);
        }
        cmd.args(["--bin", &bin]);
        cmd.current_dir(&project.root);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = log_tx.send(LogLine {
                    source: LogSource::Editor,
                    text: format!("Failed to start cargo build: {e}"),
                });
                return BuildResult::Failed(format!("spawn failed: {e}"));
            }
        };

        let mut build_output = Vec::new();
        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = log_tx.send(LogLine {
                        source: LogSource::Stderr,
                        text: line.clone(),
                    });
                    build_output.push(line);
                }
            }
        }
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = log_tx.send(LogLine {
                        source: LogSource::Stdout,
                        text: line.clone(),
                    });
                }
            }
        }

        match child.wait() {
            Ok(status) if status.success() => {
                let _ = log_tx.send(LogLine {
                    source: LogSource::Editor,
                    text: "Build succeeded.".into(),
                });

                match Self::resolved_binary(project) {
                    Ok(path) => {
                        let _ = log_tx.send(LogLine {
                            source: LogSource::Editor,
                            text: format!("Binary: {}", path.display()),
                        });
                        BuildResult::Success(path)
                    }
                    Err(e) => {
                        let _ = log_tx.send(LogLine {
                            source: LogSource::Editor,
                            text: e.clone(),
                        });
                        BuildResult::Failed(e)
                    }
                }
            }
            Ok(status) => {
                let last_lines: Vec<String> =
                    build_output.iter().rev().take(5).rev().cloned().collect();
                let summary = if last_lines.is_empty() {
                    format!("Build failed: {status}")
                } else {
                    format!("Build failed: {status}\n{}", last_lines.join("\n"))
                };
                let _ = log_tx.send(LogLine {
                    source: LogSource::Editor,
                    text: summary.clone(),
                });
                BuildResult::Failed(summary)
            }
            Err(e) => {
                let _ = log_tx.send(LogLine {
                    source: LogSource::Editor,
                    text: format!("Build process error: {e}"),
                });
                BuildResult::Failed(format!("process error: {e}"))
            }
        }
    }

    pub fn spawn(project: &ProjectConfig, shm_name: &str) -> Result<Self, String> {
        let bin_path = Self::resolved_binary(project)?;

        let mut cmd = Command::new(&bin_path);

        cmd.arg("+gamedir");
        cmd.arg(&project.gamedir);

        for cmd_str in &project.commands {
            if let Some((name, value)) = cmd_str.split_once(char::is_whitespace) {
                cmd.arg(format!("+{name}"));
                cmd.arg(value);
            } else {
                cmd.arg(format!("+{cmd_str}"));
                cmd.arg("");
            }
        }

        cmd.arg("+r_texturehandle");
        cmd.arg(shm_name);

        cmd.current_dir(&project.root);

        let display_cmd = format!("{:?}", cmd);

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn game: {e}"))?;

        let (log_tx, log_rx) = mpsc::channel();
        let (stdin_tx, stdin_rx) = mpsc::channel::<String>();

        let mut threads = Vec::new();

        if let Some(stdout) = child.stdout.take() {
            let tx = log_tx.clone();
            threads.push(thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            if tx
                                .send(LogLine {
                                    source: LogSource::Stdout,
                                    text: l,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }));
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = log_tx.clone();
            threads.push(thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            if tx
                                .send(LogLine {
                                    source: LogSource::Stderr,
                                    text: l,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }));
        }

        if let Some(stdin) = child.stdin.take() {
            threads.push(thread::spawn(move || {
                let mut stdin = stdin;
                for cmd in stdin_rx {
                    let _ = writeln!(stdin, "{cmd}");
                    let _ = stdin.flush();
                }
            }));
        }

        let pid = child.id();
        let (ipc_tx_user, ipc_rx_user) = mpsc::channel::<IpcMessage>();
        let (ipc_tx_game, ipc_rx_game) = mpsc::channel::<IpcMessage>();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        // IPC thread: connects to pipe, forwards messages bidirectionally
        {
            let log_tx = log_tx.clone();
            threads.push(thread::spawn(move || {
                let mut client: Option<IpcClient> = None;

                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        return;
                    }
                    if let Some(ref c) = client {
                        // Forward messages game → editor
                        while let Some(msg) = c.try_recv() {
                            if ipc_tx_game.send(msg).is_err() {
                                return;
                            }
                        }
                        // Forward messages editor → game
                        while let Ok(msg) = ipc_rx_user.try_recv() {
                            if c.send(&msg).is_err() {
                                return;
                            }
                        }
                    } else {
                        // Try to connect (non-blocking attempt)
                        match IpcClient::connect(pid) {
                            Ok(c) => {
                                let _ = log_tx.send(LogLine {
                                    source: LogSource::Editor,
                                    text: format!("IPC connected (pid {pid})"),
                                });
                                client = Some(c);
                            }
                            Err(_) => {}
                        }
                    }

                    thread::sleep(Duration::from_millis(16));
                }
            }));
        }

        let _ = log_tx.send(LogLine {
            source: LogSource::Editor,
            text: format!("Spawned: {display_cmd}"),
        });

        Ok(Self {
            child,
            stdin_tx: Some(stdin_tx),
            log_rx,
            _threads: threads,
            ipc_tx: Some(ipc_tx_user),
            ipc_rx: ipc_rx_game,
            shutdown: shutdown_tx,
        })
    }

    pub fn send_command(&self, command: &str) {
        if let Some(tx) = &self.stdin_tx {
            let _ = tx.send(command.to_string());
        }
    }

    pub fn try_recv_log(&self) -> Option<LogLine> {
        self.log_rx.try_recv().ok()
    }

    pub fn try_recv_ipc(&self) -> Option<IpcMessage> {
        self.ipc_rx.try_recv().ok()
    }

    pub fn send_ipc(&self, msg: &IpcMessage) {
        if let Some(tx) = &self.ipc_tx {
            let _ = tx.send(msg.clone());
        }
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for GameProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.shutdown.send(());
    }
}
