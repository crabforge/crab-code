//! Streaming output capture and command history.
//!
//! Provides [`StreamingCapture`] for real-time line-buffered capture of
//! stdout/stderr from child processes, [`OutputFilter`] for classifying and
//! highlighting output lines, and [`CommandHistory`] for recording recently
//! executed commands with their results.

use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ── Output line classification ───────────────────────────────────────

/// Classification of an output line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// Normal informational output.
    Info,
    /// Warning line.
    Warning,
    /// Error line.
    Error,
    /// Progress / status line.
    Progress,
}

impl std::fmt::Display for LineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Progress => write!(f, "progress"),
        }
    }
}

/// A single captured output line with metadata.
#[derive(Debug, Clone)]
pub struct CapturedLine {
    /// The line content (without trailing newline).
    pub text: String,
    /// Whether from stdout (true) or stderr (false).
    pub is_stdout: bool,
    /// Classification of the line.
    pub kind: LineKind,
}

// ── OutputFilter ─────────────────────────────────────────────────────

/// Classifies output lines as info, warning, error, or progress.
///
/// Uses pattern matching on common compiler/tool output formats.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct OutputFilter {
    error_patterns: Vec<String>,
    warning_patterns: Vec<String>,
    progress_patterns: Vec<String>,
}

impl OutputFilter {
    /// Create a filter with default patterns for common tools.
    #[must_use]
    pub fn new() -> Self {
        Self {
            error_patterns: vec![
                "error".into(),
                "Error".into(),
                "ERROR".into(),
                "fatal".into(),
                "FAILED".into(),
                "panic".into(),
            ],
            warning_patterns: vec![
                "warning".into(),
                "Warning".into(),
                "WARN".into(),
                "deprecated".into(),
            ],
            progress_patterns: vec![
                "Compiling".into(),
                "Downloading".into(),
                "Installing".into(),
                "Building".into(),
                "Cloning".into(),
                "Fetching".into(),
                "Resolving".into(),
                "Checking".into(),
                "Finished".into(),
                "Linking".into(),
            ],
        }
    }

    /// Add a custom error pattern.
    pub fn add_error_pattern(&mut self, pattern: impl Into<String>) {
        self.error_patterns.push(pattern.into());
    }

    /// Add a custom warning pattern.
    pub fn add_warning_pattern(&mut self, pattern: impl Into<String>) {
        self.warning_patterns.push(pattern.into());
    }

    /// Add a custom progress pattern.
    pub fn add_progress_pattern(&mut self, pattern: impl Into<String>) {
        self.progress_patterns.push(pattern.into());
    }

    /// Classify a line of output.
    #[must_use]
    pub fn classify(&self, line: &str) -> LineKind {
        // Check error first (highest priority)
        if self.error_patterns.iter().any(|p| line.contains(p.as_str())) {
            return LineKind::Error;
        }
        if self
            .warning_patterns
            .iter()
            .any(|p| line.contains(p.as_str()))
        {
            return LineKind::Warning;
        }
        if self
            .progress_patterns
            .iter()
            .any(|p| line.contains(p.as_str()))
        {
            return LineKind::Progress;
        }
        LineKind::Info
    }

    /// Classify a line and wrap it in a `CapturedLine`.
    #[must_use]
    pub fn capture_line(&self, text: String, is_stdout: bool) -> CapturedLine {
        let kind = self.classify(&text);
        CapturedLine {
            text,
            is_stdout,
            kind,
        }
    }
}

impl Default for OutputFilter {
    fn default() -> Self {
        Self::new()
    }
}

// ── StreamingCapture ─────────────────────────────────────────────────

/// Accumulates output lines in real time with classification.
///
/// Feed lines via [`push_stdout`] / [`push_stderr`], then inspect
/// the captured output or drain specific line kinds.
#[derive(Debug)]
pub struct StreamingCapture {
    lines: Vec<CapturedLine>,
    filter: OutputFilter,
    /// Maximum lines to keep (0 = unlimited).
    max_lines: usize,
}

impl StreamingCapture {
    /// Create a new capture with default filter and unlimited lines.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            filter: OutputFilter::new(),
            max_lines: 0,
        }
    }

    /// Create a capture with a custom filter.
    #[must_use]
    pub fn with_filter(filter: OutputFilter) -> Self {
        Self {
            lines: Vec::new(),
            filter,
            max_lines: 0,
        }
    }

    /// Set the maximum number of lines to retain (oldest are dropped).
    /// `0` means unlimited.
    #[must_use]
    pub fn with_max_lines(mut self, max: usize) -> Self {
        self.max_lines = max;
        self
    }

    /// Push a stdout line.
    pub fn push_stdout(&mut self, text: impl Into<String>) {
        let line = self.filter.capture_line(text.into(), true);
        self.push_line(line);
    }

    /// Push a stderr line.
    pub fn push_stderr(&mut self, text: impl Into<String>) {
        let line = self.filter.capture_line(text.into(), false);
        self.push_line(line);
    }

    fn push_line(&mut self, line: CapturedLine) {
        self.lines.push(line);
        if self.max_lines > 0 && self.lines.len() > self.max_lines {
            self.lines.remove(0);
        }
    }

    /// All captured lines.
    #[must_use]
    pub fn lines(&self) -> &[CapturedLine] {
        &self.lines
    }

    /// Total number of captured lines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether no lines have been captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Get lines of a specific kind.
    #[must_use]
    pub fn lines_of_kind(&self, kind: LineKind) -> Vec<&CapturedLine> {
        self.lines.iter().filter(|l| l.kind == kind).collect()
    }

    /// Count of error lines.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.lines.iter().filter(|l| l.kind == LineKind::Error).count()
    }

    /// Count of warning lines.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| l.kind == LineKind::Warning)
            .count()
    }

    /// Concatenate all stdout lines into a single string.
    #[must_use]
    pub fn stdout_text(&self) -> String {
        self.lines
            .iter()
            .filter(|l| l.is_stdout)
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Concatenate all stderr lines into a single string.
    #[must_use]
    pub fn stderr_text(&self) -> String {
        self.lines
            .iter()
            .filter(|l| !l.is_stdout)
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Clear all captured lines.
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

impl Default for StreamingCapture {
    fn default() -> Self {
        Self::new()
    }
}

// ── CommandHistory ───────────────────────────────────────────────────

/// A recorded command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    /// The command string.
    pub command: String,
    /// Exit code.
    pub exit_code: i32,
    /// Whether the command timed out.
    pub timed_out: bool,
    /// Duration of execution in milliseconds.
    pub duration_ms: u64,
    /// Number of stdout lines.
    pub stdout_lines: usize,
    /// Number of stderr lines.
    pub stderr_lines: usize,
    /// Number of error lines detected.
    pub error_count: usize,
    /// Number of warning lines detected.
    pub warning_count: usize,
    /// Truncated stdout preview (first N chars).
    pub stdout_preview: String,
}

/// Records recently executed commands for replay and review.
#[derive(Debug)]
pub struct CommandHistory {
    records: VecDeque<CommandRecord>,
    max_records: usize,
}

impl CommandHistory {
    /// Create a new history with the given capacity.
    #[must_use]
    pub fn new(max_records: usize) -> Self {
        Self {
            records: VecDeque::new(),
            max_records,
        }
    }

    /// Record a completed command execution.
    pub fn record(
        &mut self,
        command: impl Into<String>,
        exit_code: i32,
        timed_out: bool,
        duration: Duration,
        capture: &StreamingCapture,
    ) {
        let stdout = capture.stdout_text();
        let stdout_preview = if stdout.len() > 200 {
            format!("{}...", &stdout[..200])
        } else {
            stdout
        };

        let record = CommandRecord {
            command: command.into(),
            exit_code,
            timed_out,
            #[allow(clippy::cast_possible_truncation)]
            duration_ms: duration.as_millis() as u64,
            stdout_lines: capture.lines().iter().filter(|l| l.is_stdout).count(),
            stderr_lines: capture.lines().iter().filter(|l| !l.is_stdout).count(),
            error_count: capture.error_count(),
            warning_count: capture.warning_count(),
            stdout_preview,
        };

        if self.records.len() >= self.max_records {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    /// Record from raw values (without a capture instance).
    pub fn record_raw(&mut self, record: CommandRecord) {
        if self.records.len() >= self.max_records {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    /// All records, oldest first.
    #[must_use]
    pub fn records(&self) -> &VecDeque<CommandRecord> {
        &self.records
    }

    /// Most recent record.
    #[must_use]
    pub fn last(&self) -> Option<&CommandRecord> {
        self.records.back()
    }

    /// Number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the history is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Search records by command substring (case-insensitive).
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&CommandRecord> {
        let lower = query.to_lowercase();
        self.records
            .iter()
            .filter(|r| r.command.to_lowercase().contains(&lower))
            .collect()
    }

    /// Get only failed commands (non-zero exit code or timed out).
    #[must_use]
    pub fn failures(&self) -> Vec<&CommandRecord> {
        self.records
            .iter()
            .filter(|r| r.exit_code != 0 || r.timed_out)
            .collect()
    }

    /// Clear all records.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── LineKind ─────────────────────────────────────────────────────

    #[test]
    fn line_kind_display() {
        assert_eq!(LineKind::Info.to_string(), "info");
        assert_eq!(LineKind::Warning.to_string(), "warning");
        assert_eq!(LineKind::Error.to_string(), "error");
        assert_eq!(LineKind::Progress.to_string(), "progress");
    }

    #[test]
    fn line_kind_serde_roundtrip() {
        let json = serde_json::to_string(&LineKind::Error).unwrap();
        assert_eq!(json, r#""error""#);
        let back: LineKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LineKind::Error);
    }

    // ── OutputFilter ─────────────────────────────────────────────────

    #[test]
    fn filter_classifies_error() {
        let f = OutputFilter::new();
        assert_eq!(f.classify("error[E0308]: mismatched types"), LineKind::Error);
        assert_eq!(f.classify("fatal: not a git repository"), LineKind::Error);
    }

    #[test]
    fn filter_classifies_warning() {
        let f = OutputFilter::new();
        assert_eq!(
            f.classify("warning: unused variable `x`"),
            LineKind::Warning
        );
        assert_eq!(
            f.classify("This API is deprecated"),
            LineKind::Warning
        );
    }

    #[test]
    fn filter_classifies_progress() {
        let f = OutputFilter::new();
        assert_eq!(
            f.classify("   Compiling crab-core v0.1.0"),
            LineKind::Progress
        );
        assert_eq!(f.classify("Cloning into 'repo'..."), LineKind::Progress);
        assert_eq!(f.classify("   Finished dev target"), LineKind::Progress);
    }

    #[test]
    fn filter_classifies_info() {
        let f = OutputFilter::new();
        assert_eq!(f.classify("running 42 tests"), LineKind::Info);
        assert_eq!(f.classify("test result: ok"), LineKind::Info);
    }

    #[test]
    fn filter_error_takes_priority_over_warning() {
        let f = OutputFilter::new();
        // Contains both "error" and "warning"
        assert_eq!(
            f.classify("error: this warning is actually an error"),
            LineKind::Error
        );
    }

    #[test]
    fn filter_custom_patterns() {
        let mut f = OutputFilter::new();
        f.add_error_pattern("CRITICAL");
        f.add_warning_pattern("NOTICE");
        f.add_progress_pattern("Step");

        assert_eq!(f.classify("CRITICAL failure"), LineKind::Error);
        assert_eq!(f.classify("NOTICE: something"), LineKind::Warning);
        assert_eq!(f.classify("Step 3 of 5"), LineKind::Progress);
    }

    #[test]
    fn filter_capture_line() {
        let f = OutputFilter::new();
        let line = f.capture_line("error: oops".into(), false);
        assert_eq!(line.kind, LineKind::Error);
        assert!(!line.is_stdout);
        assert_eq!(line.text, "error: oops");
    }

    // ── StreamingCapture ─────────────────────────────────────────────

    #[test]
    fn capture_push_and_count() {
        let mut cap = StreamingCapture::new();
        cap.push_stdout("line 1");
        cap.push_stderr("error: bad");
        cap.push_stdout("line 2");
        assert_eq!(cap.len(), 3);
        assert!(!cap.is_empty());
    }

    #[test]
    fn capture_empty() {
        let cap = StreamingCapture::new();
        assert!(cap.is_empty());
        assert_eq!(cap.len(), 0);
    }

    #[test]
    fn capture_stdout_stderr_text() {
        let mut cap = StreamingCapture::new();
        cap.push_stdout("hello");
        cap.push_stderr("oops");
        cap.push_stdout("world");
        assert_eq!(cap.stdout_text(), "hello\nworld");
        assert_eq!(cap.stderr_text(), "oops");
    }

    #[test]
    fn capture_classifies_lines() {
        let mut cap = StreamingCapture::new();
        cap.push_stderr("error: failed");
        cap.push_stderr("warning: unused");
        cap.push_stdout("   Compiling foo");
        cap.push_stdout("ok");

        assert_eq!(cap.error_count(), 1);
        assert_eq!(cap.warning_count(), 1);
        assert_eq!(cap.lines_of_kind(LineKind::Progress).len(), 1);
        assert_eq!(cap.lines_of_kind(LineKind::Info).len(), 1);
    }

    #[test]
    fn capture_max_lines() {
        let mut cap = StreamingCapture::new().with_max_lines(3);
        cap.push_stdout("a");
        cap.push_stdout("b");
        cap.push_stdout("c");
        cap.push_stdout("d");
        assert_eq!(cap.len(), 3);
        // "a" should have been dropped
        assert_eq!(cap.lines()[0].text, "b");
    }

    #[test]
    fn capture_clear() {
        let mut cap = StreamingCapture::new();
        cap.push_stdout("line");
        cap.clear();
        assert!(cap.is_empty());
    }

    #[test]
    fn capture_with_custom_filter() {
        let mut filter = OutputFilter::new();
        filter.add_error_pattern("BOOM");
        let mut cap = StreamingCapture::with_filter(filter);
        cap.push_stdout("BOOM happened");
        assert_eq!(cap.error_count(), 1);
    }

    // ── CommandHistory ───────────────────────────────────────────────

    #[test]
    fn history_record_from_capture() {
        let mut history = CommandHistory::new(10);
        let mut cap = StreamingCapture::new();
        cap.push_stdout("line 1");
        cap.push_stderr("error: bad");

        history.record("cargo test", 1, false, Duration::from_millis(500), &cap);

        assert_eq!(history.len(), 1);
        let rec = history.last().unwrap();
        assert_eq!(rec.command, "cargo test");
        assert_eq!(rec.exit_code, 1);
        assert_eq!(rec.duration_ms, 500);
        assert_eq!(rec.stdout_lines, 1);
        assert_eq!(rec.stderr_lines, 1);
        assert_eq!(rec.error_count, 1);
    }

    #[test]
    fn history_evicts_oldest() {
        let mut history = CommandHistory::new(2);
        let cap = StreamingCapture::new();

        history.record("cmd1", 0, false, Duration::ZERO, &cap);
        history.record("cmd2", 0, false, Duration::ZERO, &cap);
        history.record("cmd3", 0, false, Duration::ZERO, &cap);

        assert_eq!(history.len(), 2);
        assert_eq!(history.records()[0].command, "cmd2");
        assert_eq!(history.records()[1].command, "cmd3");
    }

    #[test]
    fn history_search() {
        let mut history = CommandHistory::new(10);
        let cap = StreamingCapture::new();

        history.record("cargo test", 0, false, Duration::ZERO, &cap);
        history.record("cargo build", 0, false, Duration::ZERO, &cap);
        history.record("git status", 0, false, Duration::ZERO, &cap);

        let results = history.search("cargo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn history_search_case_insensitive() {
        let mut history = CommandHistory::new(10);
        let cap = StreamingCapture::new();
        history.record("Cargo Test", 0, false, Duration::ZERO, &cap);

        let results = history.search("cargo");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn history_failures() {
        let mut history = CommandHistory::new(10);
        let cap = StreamingCapture::new();

        history.record("ok_cmd", 0, false, Duration::ZERO, &cap);
        history.record("fail_cmd", 1, false, Duration::ZERO, &cap);
        history.record("timeout_cmd", 0, true, Duration::ZERO, &cap);

        let failures = history.failures();
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].command, "fail_cmd");
        assert_eq!(failures[1].command, "timeout_cmd");
    }

    #[test]
    fn history_clear() {
        let mut history = CommandHistory::new(10);
        let cap = StreamingCapture::new();
        history.record("cmd", 0, false, Duration::ZERO, &cap);
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn history_default_capacity() {
        let history = CommandHistory::default();
        assert!(history.is_empty());
        // Default max is 100
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn history_record_raw() {
        let mut history = CommandHistory::new(10);
        history.record_raw(CommandRecord {
            command: "echo hi".into(),
            exit_code: 0,
            timed_out: false,
            duration_ms: 10,
            stdout_lines: 1,
            stderr_lines: 0,
            error_count: 0,
            warning_count: 0,
            stdout_preview: "hi".into(),
        });
        assert_eq!(history.len(), 1);
        assert_eq!(history.last().unwrap().command, "echo hi");
    }

    #[test]
    fn history_stdout_preview_truncated() {
        let mut history = CommandHistory::new(10);
        let mut cap = StreamingCapture::new();
        let long_line = "x".repeat(300);
        cap.push_stdout(&long_line);

        history.record("cmd", 0, false, Duration::ZERO, &cap);
        let rec = history.last().unwrap();
        assert!(rec.stdout_preview.ends_with("..."));
        assert!(rec.stdout_preview.len() <= 204);
    }

    #[test]
    fn command_record_serde_roundtrip() {
        let rec = CommandRecord {
            command: "cargo test".into(),
            exit_code: 0,
            timed_out: false,
            duration_ms: 1234,
            stdout_lines: 10,
            stderr_lines: 2,
            error_count: 0,
            warning_count: 1,
            stdout_preview: "running tests...".into(),
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: CommandRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.command, "cargo test");
        assert_eq!(back.duration_ms, 1234);
    }
}
