//! Route the `log` crate from gamelib DLLs into the host logger (`v1:log:write`).

use std::sync::Once;

use deerpunk_engine_api::EngineApi;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

struct MethodDbLogger {
    api: &'static EngineApi,
}

static INSTALL: Once = Once::new();

/// Call once from `client_init` / `server_init` after [`install_engine`].
pub fn install_gamelib_logger(api: &'static EngineApi) -> Result<(), SetLoggerError> {
    let mut result = Ok(());
    INSTALL.call_once(|| {
        let logger = MethodDbLogger { api };
        result = log::set_boxed_logger(Box::new(logger));
        if result.is_ok() {
            log::set_max_level(LevelFilter::Trace);
        }
    });
    result
}

impl Log for MethodDbLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => 1,
            Level::Warn => 2,
            Level::Info => 3,
            Level::Debug => 4,
            Level::Trace => 5,
        };
        let msg = if record.target().is_empty() {
            format!("{}", record.args())
        } else {
            format!("{}: {}", record.target(), record.args())
        };
        self.api.log.write(level, &msg);
    }

    fn flush(&self) {}
}
