use log::LevelFilter;

use crate::storage::AppSettings;

/// The log levels in the order of the Log Level row in Preferences, as stored.
pub const LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

pub fn apply_settings(settings: &AppSettings) {
    log::set_max_level(level_from_str(&settings.log_level).unwrap_or(LevelFilter::Info));
}

fn level_from_str(s: &str) -> Option<LevelFilter> {
    match s {
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}
