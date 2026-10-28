//! Client demo: record / play SVC stream (+ view stamps), GoldSrc-style.
//!
//! File `{gamedir}/demos/<name>.dem`:
//! ```text
//! magic "DPD1"
//! records:
//!   f32 time_rel
//!   u8  kind   // 0 = raw SVC packet, 1 = view stamp (5×f32)
//!   u32 len LE
//!   [u8; len]
//! ```

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 4] = b"DPD1";
pub const KIND_SVC: u8 = 0;
pub const KIND_VIEW: u8 = 1;

#[derive(Clone, Copy, Debug)]
pub struct DemoView {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub eye_height: f32,
    pub fov_y: f32,
}

#[derive(Clone, Debug)]
pub enum DemoRecord {
    Svc { time: f32, packet: Vec<u8> },
    View { time: f32, view: DemoView },
}

pub struct DemoWriter {
    file: File,
    pub t0: f32,
    pub path: PathBuf,
    pub packets: u32,
}

impl DemoWriter {
    pub fn create(path: PathBuf, t0: f32) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("demos dir {}: {e}", parent.display()))?;
        }
        let mut file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(MAGIC)
            .map_err(|e| format!("write magic: {e}"))?;
        Ok(Self {
            file,
            t0,
            path,
            packets: 0,
        })
    }

    pub fn write_svc(&mut self, time_secs: f32, packet: &[u8]) -> Result<(), String> {
        self.write_record(time_secs - self.t0, KIND_SVC, packet)
    }

    pub fn write_view(&mut self, time_secs: f32, view: DemoView) -> Result<(), String> {
        let mut buf = [0u8; 20];
        buf[0..4].copy_from_slice(&view.yaw.to_le_bytes());
        buf[4..8].copy_from_slice(&view.pitch.to_le_bytes());
        buf[8..12].copy_from_slice(&view.roll.to_le_bytes());
        buf[12..16].copy_from_slice(&view.eye_height.to_le_bytes());
        buf[16..20].copy_from_slice(&view.fov_y.to_le_bytes());
        self.write_record(time_secs - self.t0, KIND_VIEW, &buf)
    }

    fn write_record(&mut self, time: f32, kind: u8, data: &[u8]) -> Result<(), String> {
        self.file
            .write_all(&time.to_le_bytes())
            .map_err(|e| format!("demo write: {e}"))?;
        self.file
            .write_all(&[kind])
            .map_err(|e| format!("demo write: {e}"))?;
        let len = data.len() as u32;
        self.file
            .write_all(&len.to_le_bytes())
            .map_err(|e| format!("demo write: {e}"))?;
        self.file
            .write_all(data)
            .map_err(|e| format!("demo write: {e}"))?;
        self.packets += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<(PathBuf, u32), String> {
        self.file.sync_all().map_err(|e| format!("demo sync: {e}"))?;
        Ok((self.path, self.packets))
    }
}

pub struct DemoPlayer {
    pub path: PathBuf,
    pub records: Vec<DemoRecord>,
    pub cursor: usize,
    pub time: f32,
    pub last_view: Option<DemoView>,
}

impl DemoPlayer {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)
            .map_err(|e| format!("demo magic: {e}"))?;
        if &magic != MAGIC {
            return Err(format!(
                "bad demo magic {:?} (want {:?})",
                magic,
                MAGIC
            ));
        }
        let mut records = Vec::new();
        loop {
            let mut tbuf = [0u8; 4];
            match file.read_exact(&mut tbuf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(format!("demo read: {e}")),
            }
            let time = f32::from_le_bytes(tbuf);
            let mut kind = [0u8; 1];
            file.read_exact(&mut kind)
                .map_err(|e| format!("demo kind: {e}"))?;
            let mut lbuf = [0u8; 4];
            file.read_exact(&mut lbuf)
                .map_err(|e| format!("demo len: {e}"))?;
            let len = u32::from_le_bytes(lbuf) as usize;
            if len > 16 * 1024 * 1024 {
                return Err(format!("demo record too large: {len}"));
            }
            let mut data = vec![0u8; len];
            file.read_exact(&mut data)
                .map_err(|e| format!("demo data: {e}"))?;
            match kind[0] {
                KIND_SVC => records.push(DemoRecord::Svc {
                    time,
                    packet: data,
                }),
                KIND_VIEW => {
                    if data.len() < 20 {
                        return Err("demo view truncated".into());
                    }
                    let yaw = f32::from_le_bytes(data[0..4].try_into().unwrap());
                    let pitch = f32::from_le_bytes(data[4..8].try_into().unwrap());
                    let roll = f32::from_le_bytes(data[8..12].try_into().unwrap());
                    let eye_height = f32::from_le_bytes(data[12..16].try_into().unwrap());
                    let fov_y = f32::from_le_bytes(data[16..20].try_into().unwrap());
                    records.push(DemoRecord::View {
                        time,
                        view: DemoView {
                            yaw,
                            pitch,
                            roll,
                            eye_height,
                            fov_y,
                        },
                    });
                }
                other => return Err(format!("demo unknown kind {other}")),
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            records,
            cursor: 0,
            time: 0.0,
            last_view: None,
        })
    }

    /// Advance clock by `dt`, return SVC packets due this frame; updates `last_view`.
    pub fn tick(&mut self, dt: f32) -> Vec<Vec<u8>> {
        self.time += dt.max(0.0);
        let mut out = Vec::new();
        while self.cursor < self.records.len() {
            let due = match &self.records[self.cursor] {
                DemoRecord::Svc { time, .. } | DemoRecord::View { time, .. } => *time,
            };
            if due > self.time {
                break;
            }
            match &self.records[self.cursor] {
                DemoRecord::Svc { packet, .. } => out.push(packet.clone()),
                DemoRecord::View { view, .. } => self.last_view = Some(*view),
            }
            self.cursor += 1;
        }
        out
    }

    pub fn finished(&self) -> bool {
        self.cursor >= self.records.len()
    }
}

pub fn sanitize_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok {
        return None;
    }
    Some(name.to_string())
}

pub fn demo_path(gamedir: &Path, name: &str) -> PathBuf {
    let mut p = gamedir.join("demos").join(name);
    if p.extension().is_none() {
        p.set_extension("dem");
    }
    p
}

deerpunk_engine_common::cvar_def!(
    "cl_demoplaying",
    "0",
    "1 while demo play is active (disables mlook)."
);
