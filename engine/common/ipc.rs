use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use interprocess::os::windows::named_pipe::{
    DuplexPipeStream, PipeListenerOptions, pipe_mode,
};

static IPC_DEBUG: AtomicBool = AtomicBool::new(false);

pub fn set_ipc_debug(enabled: bool) {
    IPC_DEBUG.store(enabled, Ordering::Relaxed);
}

fn ipc_debug() -> bool {
    IPC_DEBUG.load(Ordering::Relaxed)
}

fn log_ipc(direction: &str, msg: &IpcMessage) {
    if !ipc_debug() {
        return;
    }
    match msg {
        IpcMessage::InputState {
            mouse_dx,
            mouse_dy,
            keys,
            ..
        } => {
            eprintln!(
                "[ipc {direction}] InputState mouse({mouse_dx:.1},{mouse_dy:.1}) keys={:02X?}",
                &keys[..6]
            );
        }
        IpcMessage::ConsoleCommand(s) => {
            eprintln!("[ipc {direction}] ConsoleCommand: {s}");
        }
        IpcMessage::LogLine { source, text } => {
            eprintln!("[ipc {direction}] LogLine src={source}: {text}");
        }
        IpcMessage::FrameReady {
            width,
            height,
            frame_number,
        } => {
            eprintln!("[ipc {direction}] FrameReady {width}x{height} frame={frame_number}");
        }
        IpcMessage::EntityList { entities } => {
            eprintln!("[ipc {direction}] EntityList ({} entities)", entities.len());
        }
        IpcMessage::SharedTextureHandle {
            handle,
            width,
            height,
        } => {
            eprintln!("[ipc {direction}] SharedTextureHandle handle={handle} {width}x{height}");
        }
        IpcMessage::Ping => {
            eprintln!("[ipc {direction}] Ping");
        }
        IpcMessage::Pong => {
            eprintln!("[ipc {direction}] Pong");
        }
    }
}

// ── Wire format ──────────────────────────────────────────────
// [4 bytes: payload_len u32 LE] [1 byte: msg_type] [payload_len bytes]

const HEADER_SIZE: usize = 4;
const MAX_MSG_SIZE: usize = 1024 * 1024;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    InputState = 0x01,
    ConsoleCommand = 0x02,
    LogLine = 0x10,
    FrameReady = 0x11,
    EntityList = 0x12,
    SharedTextureHandle = 0x13,
    Ping = 0xFE,
    Pong = 0xFF,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(MsgType::InputState),
            0x02 => Some(MsgType::ConsoleCommand),
            0x10 => Some(MsgType::LogLine),
            0x11 => Some(MsgType::FrameReady),
            0x12 => Some(MsgType::EntityList),
            0x13 => Some(MsgType::SharedTextureHandle),
            0xFE => Some(MsgType::Ping),
            0xFF => Some(MsgType::Pong),
            _ => None,
        }
    }
}

// ── Messages ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum IpcMessage {
    InputState {
        mouse_dx: f32,
        mouse_dy: f32,
        mouse_buttons: u8,
        keys: [u8; 32],
    },
    ConsoleCommand(String),
    LogLine {
        source: u8,
        text: String,
    },
    FrameReady {
        width: u32,
        height: u32,
        frame_number: u32,
    },
    EntityList {
        entities: Vec<EntityInfo>,
    },
    SharedTextureHandle {
        handle: i64,
        width: u32,
        height: u32,
    },
    Ping,
    Pong,
}

#[derive(Debug, Clone)]
pub struct EntityInfo {
    pub id: u64,
    pub name: String,
    pub kind: String,
}

impl IpcMessage {
    pub fn msg_type(&self) -> MsgType {
        match self {
            IpcMessage::InputState { .. } => MsgType::InputState,
            IpcMessage::ConsoleCommand(_) => MsgType::ConsoleCommand,
            IpcMessage::LogLine { .. } => MsgType::LogLine,
            IpcMessage::FrameReady { .. } => MsgType::FrameReady,
            IpcMessage::EntityList { .. } => MsgType::EntityList,
            IpcMessage::SharedTextureHandle { .. } => MsgType::SharedTextureHandle,
            IpcMessage::Ping => MsgType::Ping,
            IpcMessage::Pong => MsgType::Pong,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        match self {
            IpcMessage::InputState {
                mouse_dx,
                mouse_dy,
                mouse_buttons,
                keys,
            } => {
                payload.extend_from_slice(&mouse_dx.to_le_bytes());
                payload.extend_from_slice(&mouse_dy.to_le_bytes());
                payload.push(*mouse_buttons);
                payload.extend_from_slice(keys);
            }
            IpcMessage::ConsoleCommand(s) => {
                payload.extend_from_slice(s.as_bytes());
            }
            IpcMessage::LogLine { source, text } => {
                payload.push(*source);
                payload.extend_from_slice(text.as_bytes());
            }
            IpcMessage::FrameReady {
                width,
                height,
                frame_number,
            } => {
                payload.extend_from_slice(&width.to_le_bytes());
                payload.extend_from_slice(&height.to_le_bytes());
                payload.extend_from_slice(&frame_number.to_le_bytes());
            }
            IpcMessage::EntityList { entities } => {
                payload.extend_from_slice(&(entities.len() as u32).to_le_bytes());
                for e in entities {
                    payload.extend_from_slice(&e.id.to_le_bytes());
                    let name_bytes = e.name.as_bytes();
                    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                    payload.extend_from_slice(name_bytes);
                    let kind_bytes = e.kind.as_bytes();
                    payload.extend_from_slice(&(kind_bytes.len() as u16).to_le_bytes());
                    payload.extend_from_slice(kind_bytes);
                }
            }
            IpcMessage::Ping | IpcMessage::Pong => {}
            IpcMessage::SharedTextureHandle {
                handle,
                width,
                height,
            } => {
                payload.extend_from_slice(&handle.to_le_bytes());
                payload.extend_from_slice(&width.to_le_bytes());
                payload.extend_from_slice(&height.to_le_bytes());
            }
        }

        let len = payload.len() as u32;
        let mut buf = Vec::with_capacity(HEADER_SIZE + 1 + payload.len());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.push(self.msg_type() as u8);
        buf.extend_from_slice(&payload);
        buf
    }

    pub fn decode(msg_type: u8, payload: &[u8]) -> Option<Self> {
        match MsgType::from_u8(msg_type)? {
            MsgType::InputState => {
                if payload.len() < 41 {
                    return None;
                }
                let mouse_dx = f32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let mouse_dy =
                    f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let mouse_buttons = payload[8];
                let mut keys = [0u8; 32];
                keys.copy_from_slice(&payload[9..41]);
                Some(IpcMessage::InputState {
                    mouse_dx,
                    mouse_dy,
                    mouse_buttons,
                    keys,
                })
            }
            MsgType::ConsoleCommand => {
                let s = String::from_utf8_lossy(payload).to_string();
                Some(IpcMessage::ConsoleCommand(s))
            }
            MsgType::LogLine => {
                if payload.is_empty() {
                    return None;
                }
                let source = payload[0];
                let text = String::from_utf8_lossy(&payload[1..]).to_string();
                Some(IpcMessage::LogLine { source, text })
            }
            MsgType::FrameReady => {
                if payload.len() < 12 {
                    return None;
                }
                let width =
                    u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let height =
                    u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let frame_number =
                    u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                Some(IpcMessage::FrameReady {
                    width,
                    height,
                    frame_number,
                })
            }
            MsgType::Ping => Some(IpcMessage::Ping),
            MsgType::Pong => Some(IpcMessage::Pong),
            MsgType::EntityList => {
                let mut cursor = 0;
                if payload.len() < 4 {
                    return None;
                }
                let count = u32::from_le_bytes([
                    payload[0],
                    payload[1],
                    payload[2],
                    payload[3],
                ]) as usize;
                cursor += 4;
                let mut entities = Vec::with_capacity(count);
                for _ in 0..count {
                    if cursor + 8 > payload.len() {
                        return None;
                    }
                    let id =
                        u64::from_le_bytes(payload[cursor..cursor + 8].try_into().ok()?);
                    cursor += 8;
                    if cursor + 2 > payload.len() {
                        return None;
                    }
                    let name_len =
                        u16::from_le_bytes([payload[cursor], payload[cursor + 1]]) as usize;
                    cursor += 2;
                    if cursor + name_len > payload.len() {
                        return None;
                    }
                    let name = String::from_utf8_lossy(&payload[cursor..cursor + name_len])
                        .to_string();
                    cursor += name_len;
                    if cursor + 2 > payload.len() {
                        return None;
                    }
                    let kind_len =
                        u16::from_le_bytes([payload[cursor], payload[cursor + 1]]) as usize;
                    cursor += 2;
                    if cursor + kind_len > payload.len() {
                        return None;
                    }
                    let kind = String::from_utf8_lossy(&payload[cursor..cursor + kind_len])
                        .to_string();
                    cursor += kind_len;
                    entities.push(EntityInfo { id, name, kind });
                }
                Some(IpcMessage::EntityList { entities })
            }
            MsgType::SharedTextureHandle => {
                if payload.len() < 16 {
                    return None;
                }
                let handle =
                    i64::from_le_bytes(payload[0..8].try_into().ok()?);
                let width =
                    u32::from_le_bytes(payload[8..12].try_into().ok()?);
                let height =
                    u32::from_le_bytes(payload[12..16].try_into().ok()?);
                Some(IpcMessage::SharedTextureHandle {
                    handle,
                    width,
                    height,
                })
            }
        }
    }
}

// ── Pipe name ────────────────────────────────────────────────

pub fn pipe_name(pid: u32) -> String {
    format!("\\\\.\\pipe\\deerpunk_{pid}")
}

// ── Read one message from any Read stream ────────────────────

fn read_one(stream: &mut dyn Read) -> io::Result<IpcMessage> {
    let mut len_buf = [0u8; HEADER_SIZE];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len > MAX_MSG_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {len} bytes"),
        ));
    }

    let mut type_buf = [0u8; 1];
    stream.read_exact(&mut type_buf)?;

    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;

    IpcMessage::decode(type_buf[0], &payload).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown message type: 0x{:02X}", type_buf[0]),
        )
    })
}

// ── Server (game side) ───────────────────────────────────────

pub struct IpcServer {
    rx: mpsc::Receiver<IpcMessage>,
    out_tx: mpsc::Sender<IpcMessage>,
}

impl IpcServer {
    pub fn start(pid: u32) -> Self {
        let name = pipe_name(pid);
        let (msg_tx, msg_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel::<IpcMessage>();

        let name_clone = name.clone();

        thread::spawn(move || {
            let listener = match PipeListenerOptions::new()
                .path(name_clone.as_str())
                .create_duplex::<pipe_mode::Bytes>()
            {
                Ok(l) => {
                    eprintln!("[ipc] pipe created: {name_clone}, waiting for editor...");
                    l
                }
                Err(e) => {
                    eprintln!("[ipc] failed to create pipe: {e}");
                    return;
                }
            };

            let stream = match listener.accept() {
                Ok(s) => {
                    eprintln!("[ipc] editor connected");
                    s
                }
                Err(e) => {
                    eprintln!("[ipc] accept failed: {e}");
                    return;
                }
            };

            let (mut reader, mut writer) = stream.split();

            // Reader thread: pipe → msg_tx
            let msg_tx_reader = msg_tx;
            thread::spawn(move || {
                loop {
                    match read_one(&mut reader) {
                        Ok(msg) => {
                            if msg_tx_reader.send(msg).is_err() {
                                break;
                            }
                        }
                        Err(ref e)
                            if e.kind() == io::ErrorKind::UnexpectedEof
                                || e.kind() == io::ErrorKind::BrokenPipe =>
                        {
                            eprintln!("[ipc] editor disconnected");
                            break;
                        }
                        Err(e) => {
                            eprintln!("[ipc] server read error: {e}");
                            break;
                        }
                    }
                }
            });

            // Writer thread: out_rx → pipe
            thread::spawn(move || {
                while let Ok(msg) = out_rx.recv() {
                    let data = msg.encode();
                    if writer.write_all(&data).is_err() {
                        eprintln!("[ipc] server write failed — editor disconnected");
                        break;
                    }
                    let _ = writer.flush();
                }
            });
        });

        Self {
            rx: msg_rx,
            out_tx,
        }
    }

    pub fn try_recv(&self) -> Option<IpcMessage> {
        let msg = self.rx.try_recv().ok()?;
        log_ipc("recv", &msg);
        Some(msg)
    }

    pub fn send(&self, msg: &IpcMessage) -> io::Result<()> {
        log_ipc("send", msg);
        if self.out_tx.send(msg.clone()).is_err() {
            return Ok(());
        }
        Ok(())
    }
}

// ── Client (editor side) ─────────────────────────────────────

pub struct IpcClient {
    rx: mpsc::Receiver<IpcMessage>,
    tx: mpsc::Sender<Vec<u8>>,
}

impl IpcClient {
    pub fn connect(pid: u32) -> io::Result<Self> {
        let name = pipe_name(pid);

        let stream = DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path(name.as_str())?;
        eprintln!("[ipc] connected to game pipe: {name}");

        let (mut reader, writer) = stream.split();

        let (tx_msg, rx) = mpsc::channel();
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();

        // Reader thread: pipe → tx_msg
        thread::spawn(move || {
            loop {
                match read_one(&mut reader) {
                    Ok(msg) => {
                        if tx_msg.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::UnexpectedEof
                            || e.kind() == io::ErrorKind::BrokenPipe =>
                    {
                        eprintln!("[ipc] game disconnected");
                        break;
                    }
                    Err(e) => {
                        eprintln!("[ipc] client read error: {e}");
                        break;
                    }
                }
            }
        });

        // Writer thread: write_rx → pipe (blocking writes in background thread)
        thread::spawn(move || {
            let mut writer = writer;
            while let Ok(data) = write_rx.recv() {
                if writer.write_all(&data).is_err() {
                    eprintln!("[ipc] client write failed — game disconnected");
                    break;
                }
                let _ = writer.flush();
            }
        });

        Ok(Self { rx, tx: write_tx })
    }

    pub fn try_recv(&self) -> Option<IpcMessage> {
        let msg = self.rx.try_recv().ok()?;
        log_ipc("recv", &msg);
        Some(msg)
    }

    pub fn send(&self, msg: &IpcMessage) -> io::Result<()> {
        log_ipc("send", msg);
        self.tx
            .send(msg.encode())
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
        Ok(())
    }
}
