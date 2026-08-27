use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

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

pub fn init(console_level: LevelFilter) -> Result<(), SetLoggerError> {
    let log_dir = Path::new("logs");
    fs::create_dir_all(log_dir).ok();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("latest.log"))
        .expect("failed to open logs/latest.log");

    let logger = Logger {
        console_level,
        file: Mutex::new(file),
    };

    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Trace);
    Ok(())
}
