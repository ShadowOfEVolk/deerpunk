use base64::{engine::general_purpose::STANDARD, Engine};
use sha1::{Digest, Sha1};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

pub struct WsConnection {
    stream: TcpStream,
    read_buf: Vec<u8>,
}

pub enum WsIncoming {
    Text(String),
    Ping(Vec<u8>),
    Close,
    WouldBlock,
}

impl WsConnection {
    pub fn accept(stream: TcpStream) -> Result<Self, String> {
        let _ = stream.set_nonblocking(true);
        let mut conn = Self {
            stream,
            read_buf: Vec::new(),
        };
        conn.perform_handshake()?;
        Ok(conn)
    }

    pub fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.send_frame(0x1, text.as_bytes())
    }

    pub fn send_pong(&mut self, payload: &[u8]) -> Result<(), String> {
        self.send_frame(0xA, payload)
    }

    pub fn read_incoming(&mut self) -> Result<WsIncoming, String> {
        loop {
            if let Some(frame) = self.try_parse_frame()? {
                return Ok(frame);
            }

            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(WsIncoming::Close),
                Ok(n) => self.read_buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return Ok(WsIncoming::WouldBlock);
                }
                Err(e) => return Err(format!("read: {e}")),
            }
        }
    }

    fn perform_handshake(&mut self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while self.read_buf.windows(4).all(|w| w != b"\r\n\r\n") {
            if std::time::Instant::now() > deadline {
                return Err("websocket handshake timeout".into());
            }
            let mut chunk = [0u8; 1024];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("connection closed during handshake".into()),
                Ok(n) => self.read_buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(format!("handshake read: {e}")),
            }
        }

        let request = std::str::from_utf8(&self.read_buf)
            .map_err(|e| format!("handshake utf8: {e}"))?
            .to_string();
        self.read_buf.clear();

        let key = request
            .lines()
            .find_map(|line| line.strip_prefix("Sec-WebSocket-Key:").map(str::trim))
            .ok_or("missing Sec-WebSocket-Key")?;

        let accept = derive_accept_key(key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        self.stream
            .write_all(response.as_bytes())
            .map_err(|e| format!("handshake write: {e}"))?;
        self.stream
            .flush()
            .map_err(|e| format!("handshake flush: {e}"))?;
        Ok(())
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mut frame = Vec::with_capacity(payload.len() + 10);
        frame.push(0x80 | opcode);
        let len = payload.len();
        if len <= 125 {
            frame.push(len as u8);
        } else if len <= 0xFFFF {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("write frame: {e}"))?;
        Ok(())
    }

    fn try_parse_frame(&mut self) -> Result<Option<WsIncoming>, String> {
        if self.read_buf.len() < 2 {
            return Ok(None);
        }

        let b0 = self.read_buf[0];
        let b1 = self.read_buf[1];
        if b0 & 0x80 == 0 {
            return Err("fragmented frames are not supported".into());
        }

        let opcode = b0 & 0x0F;
        let masked = b1 & 0x80 != 0;
        let mut payload_len = (b1 & 0x7F) as usize;
        let mut header_len = 2usize;

        if payload_len == 126 {
            if self.read_buf.len() < 4 {
                return Ok(None);
            }
            payload_len = u16::from_be_bytes([self.read_buf[2], self.read_buf[3]]) as usize;
            header_len = 4;
        } else if payload_len == 127 {
            if self.read_buf.len() < 10 {
                return Ok(None);
            }
            payload_len = u64::from_be_bytes([
                self.read_buf[2],
                self.read_buf[3],
                self.read_buf[4],
                self.read_buf[5],
                self.read_buf[6],
                self.read_buf[7],
                self.read_buf[8],
                self.read_buf[9],
            ]) as usize;
            header_len = 10;
        }

        let mask_len = if masked { 4 } else { 0 };
        let total = header_len + mask_len + payload_len;
        if self.read_buf.len() < total {
            return Ok(None);
        }

        let mask_offset = header_len;
        let payload_offset = header_len + mask_len;
        let mut payload = self.read_buf[payload_offset..payload_offset + payload_len].to_vec();
        if masked {
            let mask = &self.read_buf[mask_offset..mask_offset + 4];
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
        }
        self.read_buf.drain(..total);

        match opcode {
            0x1 => {
                let text = String::from_utf8(payload).map_err(|e| format!("text utf8: {e}"))?;
                Ok(Some(WsIncoming::Text(text)))
            }
            0x8 => Ok(Some(WsIncoming::Close)),
            0x9 => Ok(Some(WsIncoming::Ping(payload))),
            0xA => Ok(None),
            0x0 | 0x2 => Ok(None),
            other => Err(format!("unsupported opcode: {other}")),
        }
    }
}

fn derive_accept_key(key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(GUID.as_bytes());
    STANDARD.encode(hasher.finalize())
}
