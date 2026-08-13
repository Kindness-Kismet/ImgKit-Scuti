// 日志系统模块

use chrono::Local;
use env_logger::fmt::Formatter;
use log::Record;
use std::io::Write;
use std::sync::Once;

static INIT: Once = Once::new();

// 初始化日志系统
// level: 0 = 静默 (仅显示错误), 1 = 基础信息 (Info), 2 = 详细信息 (Info + Warn), 3 = 调试信息 (Debug + Trace)
pub fn init(level: u8) {
    INIT.call_once(|| {
        let log_level = match level {
            0 => log::LevelFilter::Error, // 静默模式, 仅显示错误
            1 => log::LevelFilter::Info,  // 基础信息
            2 => log::LevelFilter::Info,  // 详细信息 (与 1 相同, 但可扩展)
            3 => log::LevelFilter::Debug, // 调试信息
            _ => log::LevelFilter::Trace, // 4 及以上显示全部日志
        };

        env_logger::Builder::from_default_env()
            .filter_level(log_level)
            .format(custom_format)
            .init();
    });
}

// 自定义日志格式: [INFO] 2025/12/12 22:00:00 xxxxxx
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
