use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use serde::{Deserialize, Serialize};

const DEFAULT_EVENT_PORT: u16 = 9997;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum GameEvent {
    Kill {
        killer_entity: u64,
        victim_entity: u64,
        killer_name: String,
        victim_name: String,
    },
    RoundEnd {
        winner_name: String,
        best_players: Vec<String>,
        round_time_secs: f32,
    },
    RoundStart {
        gamemode: String,
        tank_count: u32,
        map_tiles: u32,
    },
}

pub struct EventServer {
    pub sender: mpsc::Sender<Vec<u8>>,
    active: Arc<Mutex<bool>>,
}

impl EventServer {
    pub fn new() -> (Self, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel();
        let server = Self {
            sender: tx,
            active: Arc::new(Mutex::new(false)),
        };
        (server, rx)
    }

    pub fn push_raw(&self, bytes: Vec<u8>) {
        let _ = self.sender.send(bytes);
    }

    pub fn push(&self, event: &GameEvent) {
        if let Ok(bytes) = bincode::serialize(event) {
            self.push_raw(bytes);
        }
    }

    pub fn start_server(&self, rx: mpsc::Receiver<Vec<u8>>) {
        let mut guard = self.active.lock().unwrap();
        if *guard {
            return;
        }
        *guard = true;
        drop(guard);

        let port = std::env::var("EVENT_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_EVENT_PORT);
        let addr = format!("0.0.0.0:{port}");
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[events] bind {addr} failed: {e}");
                return;
            }
        };
        eprintln!("[events] streaming on {addr}");

        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));

        let accept_clients = clients.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let _ = stream.set_nodelay(true);
                        if let Ok(mut cs) = accept_clients.lock() {
                            cs.push(stream);
                        }
                    }
                    Err(e) => {
                        eprintln!("[events] accept error: {e}");
                        break;
                    }
                }
            }
        });

        thread::spawn(move || {
            for bytes in rx {
                let mut cs = clients.lock().unwrap();
                cs.retain(|c| {
                    let mut c = c.try_clone().unwrap();
                    c.write_all(&bytes).is_ok() && c.flush().is_ok()
                });
            }
        });
    }
}
