//! Tracing/logging initialization.

use tracing_subscriber::EnvFilter;

use crate::config::{LogConfig, LogFormat};

/// Initialize the global tracing subscriber.
///
/// Safe to call more than once; subsequent calls are ignored.
pub fn init(log: &LogConfig) {
    let filter = EnvFilter::try_new(&log.level).unwrap_or_else(|_| EnvFilter::new("info"));

    match log.format {
        LogFormat::Pretty => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(true)
                .try_init();
        }
        LogFormat::Json => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .try_init();
        }
    }
}
