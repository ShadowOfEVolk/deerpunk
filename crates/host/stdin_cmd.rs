//! Terminal stdin → CmdSystem (background reader, drain on main thread).

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::sync::Mutex;
use std::thread;

use crate::cmd::CmdSystem;

static QUEUE: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Spawn a thread that reads lines from stdin into a queue.
pub fn start() {
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }

    // Detached console on Windows may have no stdin — still try.
    thread::Builder::new()
        .name("stdin-cmd".into())
        .spawn(|| {
            let stdin = std::io::stdin();
            let mut locked = stdin.lock();
            let mut line = String::new();
            let _ = writeln!(
                std::io::stderr(),
                "[cmd] stdin ready — type engine commands here (echo hi, sv_initial_scene, …)"
            );
            loop {
                line.clear();
                match locked.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with("//") {
                            continue;
                        }
                        if let Ok(mut q) = QUEUE.lock() {
                            q.push_back(trimmed.to_string());
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .ok();
}

/// Push queued stdin lines into cmd and execute.
pub fn drain(cmd: &CmdSystem) {
    let lines: Vec<String> = {
        let Ok(mut q) = QUEUE.lock() else {
            return;
        };
        q.drain(..).collect()
    };
    for line in lines {
        eprintln!("] {line}");
        cmd.add_command(line);
    }
    cmd.process_buffer();
}
