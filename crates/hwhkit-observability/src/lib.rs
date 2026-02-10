use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub level: String,
    pub json: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            json: false,
        }
    }
}

pub fn init_logging(config: &LoggingConfig) -> Result<(), String> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.level))
        .map_err(|e| e.to_string())?;

    let fmt_layer = fmt::layer()
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .try_init();

    Ok(())
}
