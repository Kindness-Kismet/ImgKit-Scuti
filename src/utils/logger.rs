// Log system module

use chrono::Local;
use env_logger::fmt::Formatter;
use log::Record;
use std::io::Write;
use std::sync::Once;

static INIT: Once = Once::new();

// Initialize the log system
// level: 0 = Silent (only errors are displayed), 1 = Basic information (Info), 2 = Detailed information (Info + Warn), 3 = Debug information (Debug + Trace)
pub fn init(level: u8) {
    INIT.call_once(|| {
        let log_level = match level {
            0 => log::LevelFilter::Error, // Silent mode, only errors are displayed
            1 => log::LevelFilter::Info,  // Basic information
            2 => log::LevelFilter::Info,  // Details (same as 1, but extendable)
            3 => log::LevelFilter::Debug, // debugging information
            _ => log::LevelFilter::Trace, // 4 and above show all logs
        };

        env_logger::Builder::from_default_env()
            .filter_level(log_level)
            .format(custom_format)
            .init();
    });
}

// Custom log format: [INFO] 2025/12/12 22:00:00 xxxxxx
fn custom_format(buf: &mut Formatter, record: &Record) -> std::io::Result<()> {
    let timestamp = Local::now().format("%Y/%m/%d %H:%M:%S");
    let level = match record.level() {
        log::Level::Error => "ERROR",
        log::Level::Warn => "WARN",
        log::Level::Info => "INFO",
        log::Level::Debug => "DEBUG",
        log::Level::Trace => "TRACE",
    };

    writeln!(buf, "[{}] {} {}", level, timestamp, record.args())
}
