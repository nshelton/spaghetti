//! Console/log viewer state.

use std::sync::{Arc, Mutex};

use tracing::Level;

use crate::log_capture::LogBuffer;

/// Console panel state.
pub struct ConsoleState {
    /// Whether the console panel is visible.
    pub show_console: bool,
    /// Shared log buffer (also written to by the tracing layer).
    pub log_buffer: Arc<Mutex<LogBuffer>>,
    /// Severity threshold for the console display.
    pub console_level_filter: Level,
}
