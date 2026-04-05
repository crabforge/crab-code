//! MCP logging support — `logging/setLevel`.
//!
//! Provides [`McpLogLevel`] (RFC 5424 severity levels) and [`McpLogger`]
//! for managing the MCP logging level and emitting log notifications.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};

/// MCP log levels matching RFC 5424 syslog severity levels.
///
/// Ordered from most verbose (Debug) to most critical (Emergency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpLogLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

impl McpLogLevel {
    /// All levels in order from most verbose to most critical.
    pub const ALL: [Self; 8] = [
        Self::Debug,
        Self::Info,
        Self::Notice,
        Self::Warning,
        Self::Error,
        Self::Critical,
        Self::Alert,
        Self::Emergency,
    ];
}

impl fmt::Display for McpLogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Debug => write!(f, "debug"),
            Self::Info => write!(f, "info"),
            Self::Notice => write!(f, "notice"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Critical => write!(f, "critical"),
            Self::Alert => write!(f, "alert"),
            Self::Emergency => write!(f, "emergency"),
        }
    }
}

impl std::str::FromStr for McpLogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "notice" => Ok(Self::Notice),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            "critical" => Ok(Self::Critical),
            "alert" => Ok(Self::Alert),
            "emergency" => Ok(Self::Emergency),
            other => Err(format!("unknown log level: {other}")),
        }
    }
}

/// A log entry emitted via MCP `notifications/message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpLogEntry {
    pub level: McpLogLevel,
    pub logger: String,
    pub data: serde_json::Value,
}

impl McpLogEntry {
    /// Create a text log entry.
    #[must_use]
    pub fn text(level: McpLogLevel, logger: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level,
            logger: logger.into(),
            data: serde_json::Value::String(message.into()),
        }
    }

    /// Create a structured log entry.
    #[must_use]
    pub fn structured(level: McpLogLevel, logger: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            level,
            logger: logger.into(),
            data,
        }
    }
}

/// MCP logger that tracks the current log level and collects entries.
///
/// Only entries at or above the configured level are recorded.
#[derive(Debug, Clone)]
pub struct McpLogger {
    inner: Arc<Mutex<LoggerInner>>,
}

#[derive(Debug)]
struct LoggerInner {
    level: McpLogLevel,
    entries: Vec<McpLogEntry>,
    max_entries: usize,
}

impl McpLogger {
    /// Create a new logger with the specified initial level.
    #[must_use]
    pub fn new(level: McpLogLevel, max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LoggerInner {
                level,
                entries: Vec::new(),
                max_entries,
            })),
        }
    }

    /// Get the current log level.
    #[must_use]
    pub fn level(&self) -> McpLogLevel {
        self.inner.lock().unwrap().level
    }

    /// Set the log level (corresponds to `logging/setLevel`).
    pub fn set_level(&self, level: McpLogLevel) {
        self.inner.lock().unwrap().level = level;
    }

    /// Log an entry. Returns true if it was recorded (at or above current level).
    pub fn log(&self, entry: McpLogEntry) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if entry.level < inner.level {
            return false;
        }
        inner.entries.push(entry);
        // Evict oldest if over capacity
        if inner.entries.len() > inner.max_entries {
            let excess = inner.entries.len() - inner.max_entries;
            inner.entries.drain(..excess);
        }
        true
    }

    /// Convenience: log a text message.
    pub fn log_text(
        &self,
        level: McpLogLevel,
        logger: impl Into<String>,
        message: impl Into<String>,
    ) -> bool {
        self.log(McpLogEntry::text(level, logger, message))
    }

    /// Get all recorded entries.
    #[must_use]
    pub fn entries(&self) -> Vec<McpLogEntry> {
        self.inner.lock().unwrap().entries.clone()
    }

    /// Number of recorded entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().entries.len()
    }

    /// Whether no entries have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().entries.is_empty()
    }

    /// Get entries at or above a specific level.
    #[must_use]
    pub fn entries_at_level(&self, min_level: McpLogLevel) -> Vec<McpLogEntry> {
        self.inner
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.level >= min_level)
            .cloned()
            .collect()
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.inner.lock().unwrap().entries.clear();
    }
}

impl Default for McpLogger {
    fn default() -> Self {
        Self::new(McpLogLevel::Info, 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_ordering() {
        assert!(McpLogLevel::Debug < McpLogLevel::Info);
        assert!(McpLogLevel::Info < McpLogLevel::Notice);
        assert!(McpLogLevel::Notice < McpLogLevel::Warning);
        assert!(McpLogLevel::Warning < McpLogLevel::Error);
        assert!(McpLogLevel::Error < McpLogLevel::Critical);
        assert!(McpLogLevel::Critical < McpLogLevel::Alert);
        assert!(McpLogLevel::Alert < McpLogLevel::Emergency);
    }

    #[test]
    fn log_level_display_and_parse() {
        for level in McpLogLevel::ALL {
            let s = level.to_string();
            let back: McpLogLevel = s.parse().unwrap();
            assert_eq!(level, back);
        }
        assert!("unknown".parse::<McpLogLevel>().is_err());
    }

    #[test]
    fn log_level_serde_roundtrip() {
        for level in McpLogLevel::ALL {
            let json = serde_json::to_string(&level).unwrap();
            let back: McpLogLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, back);
        }
    }

    #[test]
    fn log_entry_text() {
        let entry = McpLogEntry::text(McpLogLevel::Info, "mcp-server", "connected");
        assert_eq!(entry.level, McpLogLevel::Info);
        assert_eq!(entry.logger, "mcp-server");
        assert_eq!(entry.data, serde_json::Value::String("connected".into()));
    }

    #[test]
    fn log_entry_structured() {
        let data = serde_json::json!({"key": "value", "count": 42});
        let entry = McpLogEntry::structured(McpLogLevel::Warning, "test", data.clone());
        assert_eq!(entry.data, data);
    }

    #[test]
    fn log_entry_serde_roundtrip() {
        let entry = McpLogEntry::text(McpLogLevel::Error, "srv", "oops");
        let json = serde_json::to_string(&entry).unwrap();
        let back: McpLogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.level, McpLogLevel::Error);
        assert_eq!(back.logger, "srv");
    }

    #[test]
    fn logger_filters_below_level() {
        let logger = McpLogger::new(McpLogLevel::Warning, 100);
        assert!(!logger.log_text(McpLogLevel::Debug, "t", "debug msg"));
        assert!(!logger.log_text(McpLogLevel::Info, "t", "info msg"));
        assert!(logger.log_text(McpLogLevel::Warning, "t", "warn msg"));
        assert!(logger.log_text(McpLogLevel::Error, "t", "error msg"));
        assert_eq!(logger.len(), 2);
    }

    #[test]
    fn logger_set_level() {
        let logger = McpLogger::new(McpLogLevel::Error, 100);
        assert!(!logger.log_text(McpLogLevel::Warning, "t", "warn"));
        logger.set_level(McpLogLevel::Debug);
        assert!(logger.log_text(McpLogLevel::Debug, "t", "debug now ok"));
        assert_eq!(logger.level(), McpLogLevel::Debug);
    }

    #[test]
    fn logger_entries_at_level() {
        let logger = McpLogger::new(McpLogLevel::Debug, 100);
        logger.log_text(McpLogLevel::Debug, "t", "d");
        logger.log_text(McpLogLevel::Info, "t", "i");
        logger.log_text(McpLogLevel::Error, "t", "e");
        let errors = logger.entries_at_level(McpLogLevel::Error);
        assert_eq!(errors.len(), 1);
        let info_up = logger.entries_at_level(McpLogLevel::Info);
        assert_eq!(info_up.len(), 2);
    }

    #[test]
    fn logger_capacity_eviction() {
        let logger = McpLogger::new(McpLogLevel::Debug, 3);
        for i in 0..5 {
            logger.log_text(McpLogLevel::Info, "t", format!("msg {i}"));
        }
        assert_eq!(logger.len(), 3);
        let entries = logger.entries();
        assert!(entries[0].data.as_str().unwrap().contains("msg 2"));
    }

    #[test]
    fn logger_clear() {
        let logger = McpLogger::default();
        logger.log_text(McpLogLevel::Info, "t", "hi");
        assert!(!logger.is_empty());
        logger.clear();
        assert!(logger.is_empty());
    }

    #[test]
    fn logger_thread_safe() {
        let logger = McpLogger::new(McpLogLevel::Debug, 1000);
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let lg = logger.clone();
                std::thread::spawn(move || {
                    for j in 0..25 {
                        lg.log_text(McpLogLevel::Info, "t", format!("{i}-{j}"));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(logger.len(), 100);
    }

    #[test]
    fn logger_default() {
        let logger = McpLogger::default();
        assert_eq!(logger.level(), McpLogLevel::Info);
        assert!(logger.is_empty());
    }
}
