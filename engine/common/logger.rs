use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

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
pub fn init(
    console_level: LevelFilter,
    log_dir: impl AsRef<Path>,
) -> Result<(), SetLoggerError> {
    let log_dir = log_dir.as_ref();
    fs::create_dir_all(log_dir).ok();
    let path = log_dir.join("latest.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
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
