//! Tracing layer that captures log events for display in the UI console.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Maximum number of log entries retained in the buffer.
const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// A single captured log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Formatted timestamp string.
    pub timestamp: String,
    /// Severity level.
    pub level: Level,
    /// The log message.
    pub message: String,
}

/// Ring buffer of log entries, dropping the oldest when full.
#[derive(Debug)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    max_entries: usize,
}

impl LogBuffer {
    /// Create a new buffer with the default capacity.
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    /// Create a new buffer with a custom capacity.
    #[cfg(test)]
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries,
        }
    }

    /// Push a new entry, evicting the oldest if at capacity.
    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Read-only access to all entries.
    pub fn entries(&self) -> &VecDeque<LogEntry> {
        &self.entries
    }

    /// Number of entries currently stored.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// A [`tracing_subscriber::Layer`] that captures events into a shared [`LogBuffer`].
#[derive(Clone)]
pub struct LogCaptureLayer {
    buffer: Arc<Mutex<LogBuffer>>,
}

impl LogCaptureLayer {
    /// Create a new capture layer backed by the given buffer.
    pub fn new(buffer: Arc<Mutex<LogBuffer>>) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogCaptureLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let now = std::time::SystemTime::now();
        let timestamp = format_timestamp(now);

        let entry = LogEntry {
            timestamp,
            level: *event.metadata().level(),
            message: visitor.message,
        };

        if let Ok(mut buf) = self.buffer.lock() {
            buf.push(entry);
        }
    }
}

/// Formats a [`SystemTime`] as `HH:MM:SS`.
fn format_timestamp(time: std::time::SystemTime) -> String {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// Visitor that extracts the `message` field from tracing events.
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.append_field(field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            let formatted = format!("{value:?}");
            self.append_field(field.name(), &formatted);
        }
    }
}

impl MessageVisitor {
    fn append_field(&mut self, name: &str, value: &str) {
        if !self.message.is_empty() {
            let _ = write!(self.message, " {name}={value}");
        } else {
            let _ = write!(self.message, "{name}={value}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    #[test]
    fn log_capture_records_events() {
        let buffer = Arc::new(Mutex::new(LogBuffer::new()));
        let layer = LogCaptureLayer::new(Arc::clone(&buffer));

        let _guard = tracing_subscriber::registry().with(layer).set_default();

        tracing::info!("hello world");
        tracing::warn!("something fishy");
        tracing::error!("bad thing");

        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.entries()[0].level, Level::INFO);
        assert_eq!(buf.entries()[1].level, Level::WARN);
        assert_eq!(buf.entries()[2].level, Level::ERROR);
        assert!(buf.entries()[0].message.contains("hello world"));
        assert!(buf.entries()[2].message.contains("bad thing"));
    }

    #[test]
    fn buffer_drops_oldest_when_full() {
        let buffer = Arc::new(Mutex::new(LogBuffer::with_capacity(3)));
        let layer = LogCaptureLayer::new(Arc::clone(&buffer));

        let _guard = tracing_subscriber::registry().with(layer).set_default();

        tracing::info!("one");
        tracing::info!("two");
        tracing::info!("three");
        tracing::info!("four");

        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 3);
        // "one" should have been evicted
        assert!(buf.entries()[0].message.contains("two"));
        assert!(buf.entries()[1].message.contains("three"));
        assert!(buf.entries()[2].message.contains("four"));
    }
}
