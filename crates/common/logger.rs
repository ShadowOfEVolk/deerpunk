use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Fixed line capacity (bytes, no trailing NUL required for readers).
pub const LOG_LINE_CAP: usize = 256;
/// Ring slots — 512 × 256 ≈ 128 KiB, no heap growth.
pub const LOG_RING_CAP: usize = 512;

struct LogRing {
    lines: [[u8; LOG_LINE_CAP]; LOG_RING_CAP],
    lens: [u16; LOG_RING_CAP],
    /// Next write index.
    head: usize,
    /// Occupied slots (≤ LOG_RING_CAP).
    count: usize,
}

impl LogRing {
    const fn new() -> Self {
        Self {
            lines: [[0u8; LOG_LINE_CAP]; LOG_RING_CAP],
            lens: [0u16; LOG_RING_CAP],
            head: 0,
            count: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(LOG_LINE_CAP);
        let slot = self.head;
        self.lines[slot][..n].copy_from_slice(&bytes[..n]);
        if n < LOG_LINE_CAP {
            // leave rest stale; length gates reads
        }
        self.lens[slot] = n as u16;
        self.head = (self.head + 1) % LOG_RING_CAP;
        if self.count < LOG_RING_CAP {
            self.count += 1;
        }
    }

    fn for_each(&self, mut f: impl FnMut(&[u8])) {
        let start = if self.count < LOG_RING_CAP {
            0
        } else {
            self.head
        };
        for i in 0..self.count {
            let idx = (start + i) % LOG_RING_CAP;
            let len = self.lens[idx] as usize;
            f(&self.lines[idx][..len]);
        }
    }
}

static LOG_RING: Mutex<LogRing> = Mutex::new(LogRing::new());

/// Iterate log lines oldest→newest (for in-game console). Callback must not log.
pub fn for_each_line(f: impl FnMut(&[u8])) {
    if let Ok(ring) = LOG_RING.lock() {
        ring.for_each(f);
    }
}

/// Push a raw line into the ring (truncated). No heap alloc inside.
pub fn push_line_bytes(bytes: &[u8]) {
    if let Ok(mut ring) = LOG_RING.lock() {
        // Strip trailing newline for display compactness.
        let end = bytes
            .iter()
            .rposition(|&b| b != b'\n' && b != b'\r')
            .map(|i| i + 1)
            .unwrap_or(0);
        ring.push(&bytes[..end]);
    }
}

struct Logger {
    console_level: LevelFilter,
    file: Mutex<File>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.console_level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let millis = now.subsec_millis();

        let t = time_from_epoch(secs);
        let level = record.level();
        let target = record.metadata().target();
        let args = record.args();

        let line = format!(
            "[{:02}:{:02}:{:02}.{:03}] [{:<5}] [{}] {}\n",
            t.hour, t.min, t.sec, millis, level, target, args
        );

        push_line_bytes(line.as_bytes());

        if record.level() <= LevelFilter::Warn {
            eprint!("{}", line);
        } else {
            print!("{}", line);
        }

        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

struct TimeParts {
    hour: u32,
    min: u32,
    sec: u32,
}

fn time_from_epoch(secs: u64) -> TimeParts {
    let t = (secs % 86400) as u32;
    TimeParts {
        hour: t / 3600,
        min: (t % 3600) / 60,
        sec: t % 60,
    }
}

/// Open `{log_dir}/latest.log` (typically `{gamedir}/logs`).
///
/// If `latest.log` already exists, it is renamed to `{createdate}.log`
/// (`YYYY-MM-DD_HH-MM-SS.log` from the file's creation time) before a fresh
/// `latest.log` is created.
pub fn init(
    console_level: LevelFilter,
    log_dir: impl AsRef<Path>,
) -> Result<(), SetLoggerError> {
    let log_dir = log_dir.as_ref();
    fs::create_dir_all(log_dir).ok();
    let path = log_dir.join("latest.log");
    rotate_latest_log(&path);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display()));

    let logger = Logger {
        console_level,
        file: Mutex::new(file),
    };

    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Trace);
    Ok(())
}

/// Move existing `latest.log` → `{createdate}.log` (no-op if missing).
fn rotate_latest_log(latest: &Path) {
    if !latest.is_file() {
        return;
    }
    let Some(dir) = latest.parent() else {
        return;
    };
    let stamp = file_createdate(latest);
    let mut dest = dir.join(format!("{stamp}.log"));
    let mut n = 1u32;
    while dest.exists() {
        dest = dir.join(format!("{stamp}_{n}.log"));
        n += 1;
    }
    if fs::rename(latest, &dest).is_err() {
        let _ = fs::remove_file(latest);
    }
}

/// UTC `YYYY-MM-DD_HH-MM-SS` from file creation time (mtime fallback).
fn file_createdate(path: &Path) -> String {
    let meta = fs::metadata(path).ok();
    let created = meta
        .as_ref()
        .and_then(|m| m.created().ok())
        .or_else(|| meta.as_ref().and_then(|m| m.modified().ok()))
        .unwrap_or_else(SystemTime::now);
    format_utc_stamp(created)
}

fn format_utc_stamp(t: SystemTime) -> String {
    let Ok(duration) = t.duration_since(UNIX_EPOCH) else {
        return "1970-01-01_00-00-00".into();
    };
    let secs = duration.as_secs() as i64;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400) as u32;
    let (y, m, d) = civil_from_days(days);
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;
    format!("{y:04}-{m:02}-{d:02}_{hour:02}-{min:02}-{sec:02}")
}

/// Howard Hinnant civil_from_days (UTC day number → Y-M-D).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
