//! Minimal file-backed logger. Writes to `{app_data_dir}/voxalic.log`.
//!
//! Log level defaults to `Info` — `warn!` and `error!` are always recorded.
//! Set `RUST_LOG=debug` for verbose output.

use chrono::Local;
use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

struct FileLogger {
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(
            self.file.lock().unwrap(),
            "[{} {}] {}",
            ts,
            record.level(),
            record.args()
        );
    }

    fn flush(&self) {
        let _ = self.file.lock().unwrap().flush();
    }

    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= LevelFilter::Info
    }
}

pub fn init(app_data_dir: &Path) {
    let log_path = app_data_dir.join("voxalic.log");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .unwrap_or_else(|e| {
            eprintln!("[log] cannot open {}: {e}", log_path.display());
            std::process::exit(1);
        });

    let logger = FileLogger {
        file: Mutex::new(file),
    };

    log::set_logger(Box::leak(Box::new(logger)))
        .map(|()| log::set_max_level(LevelFilter::Info))
        .unwrap_or_else(|e| eprintln!("[log] init failed: {e}"));
}
