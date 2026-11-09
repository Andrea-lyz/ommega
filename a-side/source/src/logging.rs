use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use log::LevelFilter;

const DEFAULT_LOG_PATH: &str = "/data/misc/keystore/ommega/logs/keymint.log";
const PATTERN: &str = "{d(%Y-%m-%d %H:%M:%S %Z)(utc)} [{h({l})}] {M} - {m}{n}";
/// Optional rotation override written by the WebUI diagnostics page.
const ROTATION_FILE: &str = "/data/adb/ommega/ommegadata/data/log-rotation.conf";

static LOGGER_INIT: OnceLock<()> = OnceLock::new();

pub fn init_logger() {
    let _ = LOGGER_INIT.get_or_init(|| {
        if let Err(error) = init_logger_inner() {
            eprintln!("keymint logging failed to initialize: {error:#}");
        }
    });
}

fn init_logger_inner() -> Result<()> {
    let config = android_logger::Config::default()
        .with_max_level(LevelFilter::Trace)
        .with_tag("ommega");

    let android_logger = android_logger::AndroidLogger::new(config);

    let max_size = kmr_common::runtime::logging::max_log_size_from_file(Path::new(ROTATION_FILE))
        .unwrap_or(kmr_common::runtime::logging::DEFAULT_MAX_LOG_SIZE_BYTES);
    let (config, file_logging_ready) =
        kmr_common::runtime::logging::build_console_file_config_with_size(
            DEFAULT_LOG_PATH,
            PATTERN,
            LevelFilter::Trace,
            "keymint logging",
            max_size,
        )?;
    let log4rs = log4rs::Logger::new(config);

    multi_log::MultiLogger::init(
        vec![Box::new(android_logger), Box::new(log4rs)],
        log::Level::Trace,
    )?;
    log::set_max_level(LevelFilter::Debug);

    if file_logging_ready {
        log::info!(
            "file logging enabled at {} with level {:?} rotation={}B",
            DEFAULT_LOG_PATH,
            LevelFilter::Debug,
            max_size
        );
    }

    Ok(())
}
