//! Progress parsing for common command-line tools.
//!
//! Extracts structured progress information from the output of tools like
//! `cargo build`, `git clone`, `npm install`, etc.

use serde::{Deserialize, Serialize};

// ── Progress event ───────────────────────────────────────────────────

/// A parsed progress update from command output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    /// Source tool (e.g. "cargo", "git", "npm").
    pub tool: String,
    /// Current phase or action (e.g. "Compiling", "Downloading").
    pub phase: String,
    /// Optional target name (e.g. crate name, package name).
    pub target: Option<String>,
    /// Current step (if deterministic).
    pub current: Option<u64>,
    /// Total steps (if known).
    pub total: Option<u64>,
    /// Percentage (0-100), computed from current/total or parsed directly.
    pub percent: Option<u8>,
}

impl ProgressEvent {
    /// Create a simple progress event without numeric progress.
    #[must_use]
    pub fn phase(tool: impl Into<String>, phase: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            phase: phase.into(),
            target: None,
            current: None,
            total: None,
            percent: None,
        }
    }

    /// Set the target name.
    #[must_use]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Set numeric progress.
    #[must_use]
    pub fn with_progress(mut self, current: u64, total: u64) -> Self {
        self.current = Some(current);
        self.total = Some(total);
        if total > 0 {
            self.percent = Some((current * 100 / total).min(100) as u8);
        }
        self
    }

    /// Set percentage directly.
    #[must_use]
    pub fn with_percent(mut self, pct: u8) -> Self {
        self.percent = Some(pct.min(100));
        self
    }

    /// Whether this progress event indicates completion.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.percent == Some(100)
            || self
                .current
                .zip(self.total)
                .is_some_and(|(c, t)| t > 0 && c >= t)
    }
}

// ── ProgressParser ───────────────────────────────────────────────────

/// Parses progress from output lines of common tools.
#[derive(Debug, Clone)]
pub struct ProgressParser {
    /// Tracks cargo compilation progress: (`compiled_count`, `known_total`).
    cargo_state: Option<(u64, Option<u64>)>,
}

impl ProgressParser {
    /// Create a new parser.
    #[must_use]
    pub fn new() -> Self {
        Self { cargo_state: None }
    }

    /// Reset internal tracking state.
    pub fn reset(&mut self) {
        self.cargo_state = None;
    }

    /// Try to parse a progress event from a line of output.
    /// Returns `None` if the line doesn't contain recognizable progress.
    #[must_use]
    pub fn parse_line(&mut self, line: &str) -> Option<ProgressEvent> {
        let trimmed = line.trim();

        // Try each tool parser in order
        self.parse_cargo(trimmed)
            .or_else(|| Self::parse_git(trimmed))
            .or_else(|| Self::parse_npm(trimmed))
            .or_else(|| Self::parse_generic_percent(trimmed))
    }

    /// Parse cargo output lines.
    fn parse_cargo(&mut self, line: &str) -> Option<ProgressEvent> {
        // "   Compiling crate-name v0.1.0 (/path)"
        // "   Checking crate-name v0.1.0"
        // "    Finished dev [unoptimized + debuginfo] target(s) in 2.34s"
        // "   Downloading crates ..."
        // "  Downloaded serde v1.0.0"

        let cargo_phases = [
            "Compiling",
            "Checking",
            "Downloading",
            "Downloaded",
            "Finished",
            "Linking",
            "Building",
            "Fresh",
        ];

        for phase in &cargo_phases {
            if let Some(rest) = trimmed_after_phase(line, phase) {
                if *phase == "Finished" {
                    self.cargo_state = None;
                    return Some(
                        ProgressEvent::phase("cargo", *phase)
                            .with_target(rest)
                            .with_percent(100),
                    );
                }

                if *phase == "Compiling" || *phase == "Checking" {
                    let count = self
                        .cargo_state
                        .get_or_insert((0, None));
                    count.0 += 1;
                    let target = rest.split_whitespace().next().unwrap_or(rest);
                    let mut evt =
                        ProgressEvent::phase("cargo", *phase).with_target(target);
                    if let Some(total) = count.1 {
                        evt = evt.with_progress(count.0, total);
                    }
                    return Some(evt);
                }

                let target = rest.split_whitespace().next().unwrap_or(rest);
                return Some(ProgressEvent::phase("cargo", *phase).with_target(target));
            }
        }

        None
    }

    /// Parse git output lines.
    fn parse_git(line: &str) -> Option<ProgressEvent> {
        // "Cloning into 'repo'..."
        if let Some(rest) = trimmed_after_phase(line, "Cloning into") {
            let repo = rest.trim_matches('\'').trim_matches('.').trim_matches('\'');
            return Some(ProgressEvent::phase("git", "Cloning").with_target(repo));
        }

        // "Receiving objects:  45% (123/274)"
        // "Resolving deltas:  30% (50/167)"
        if line.contains("objects:") || line.contains("deltas:") {
            let phase = if line.contains("objects:") {
                "Receiving objects"
            } else {
                "Resolving deltas"
            };

            if let Some(pct) = extract_percentage(line) {
                let mut evt = ProgressEvent::phase("git", phase).with_percent(pct);
                if let Some((cur, tot)) = extract_fraction(line) {
                    evt.current = Some(cur);
                    evt.total = Some(tot);
                    // Keep the explicit percentage from the line, don't recalculate
                }
                return Some(evt);
            }
        }

        // "Enumerating objects: 274, done."
        if let Some(rest) = trimmed_after_phase(line, "Enumerating objects:") {
            return Some(ProgressEvent::phase("git", "Enumerating objects").with_target(rest));
        }

        // "Counting objects: ..."
        if trimmed_after_phase(line, "Counting objects:").is_some() {
            return Some(ProgressEvent::phase("git", "Counting objects"));
        }

        None
    }

    /// Parse npm output lines.
    fn parse_npm(line: &str) -> Option<ProgressEvent> {
        // "npm warn deprecated ..."
        // "added 150 packages in 3s"
        // "npm install ..."

        if line.starts_with("added ") && line.contains("packages") {
            return Some(
                ProgressEvent::phase("npm", "installed")
                    .with_target(line)
                    .with_percent(100),
            );
        }

        if line.starts_with("npm warn") {
            return Some(ProgressEvent::phase("npm", "warning").with_target(line));
        }

        None
    }

    /// Parse generic percentage output (e.g. "Progress: 45%").
    fn parse_generic_percent(line: &str) -> Option<ProgressEvent> {
        if let Some(pct) = extract_percentage(line) {
            return Some(ProgressEvent::phase("unknown", "progress").with_percent(pct));
        }
        None
    }
}

impl Default for ProgressParser {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helper functions ─────────────────────────────────────────────────

/// Extract text after a phase keyword, handling optional leading whitespace.
/// Returns `None` if the phase is not found.
fn trimmed_after_phase<'a>(line: &'a str, phase: &str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix(phase)
        .map(str::trim_start)
}

/// Extract a percentage from a string like "45%" or " 45%".
fn extract_percentage(s: &str) -> Option<u8> {
    // Look for pattern: digits followed by %
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'%' {
                let num: u64 = s[start..i].parse().ok()?;
                return Some(num.min(100) as u8);
            }
        }
        i += 1;
    }
    None
}

/// Extract a fraction like (123/274) from a string.
fn extract_fraction(s: &str) -> Option<(u64, u64)> {
    let open = s.find('(')?;
    let close = s.find(')')?;
    if close <= open {
        return None;
    }
    let inner = &s[open + 1..close];
    let slash = inner.find('/')?;
    let current = inner[..slash].trim().parse().ok()?;
    let total = inner[slash + 1..].trim().parse().ok()?;
    Some((current, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ProgressEvent ────────────────────────────────────────────────

    #[test]
    fn event_phase_only() {
        let evt = ProgressEvent::phase("cargo", "Compiling");
        assert_eq!(evt.tool, "cargo");
        assert_eq!(evt.phase, "Compiling");
        assert!(evt.target.is_none());
        assert!(evt.percent.is_none());
        assert!(!evt.is_complete());
    }

    #[test]
    fn event_with_progress() {
        let evt = ProgressEvent::phase("git", "Receiving")
            .with_progress(50, 100);
        assert_eq!(evt.current, Some(50));
        assert_eq!(evt.total, Some(100));
        assert_eq!(evt.percent, Some(50));
        assert!(!evt.is_complete());
    }

    #[test]
    fn event_is_complete_by_percent() {
        let evt = ProgressEvent::phase("test", "done").with_percent(100);
        assert!(evt.is_complete());
    }

    #[test]
    fn event_is_complete_by_progress() {
        let evt = ProgressEvent::phase("test", "done").with_progress(10, 10);
        assert!(evt.is_complete());
    }

    #[test]
    fn event_percent_clamped() {
        let evt = ProgressEvent::phase("test", "x").with_percent(200);
        assert_eq!(evt.percent, Some(100));
    }

    #[test]
    fn event_serde_roundtrip() {
        let evt = ProgressEvent::phase("cargo", "Compiling")
            .with_target("serde")
            .with_progress(3, 10);
        let json = serde_json::to_string(&evt).unwrap();
        let back: ProgressEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool, "cargo");
        assert_eq!(back.target.as_deref(), Some("serde"));
        assert_eq!(back.percent, Some(30));
    }

    // ── Cargo parsing ────────────────────────────────────────────────

    #[test]
    fn parse_cargo_compiling() {
        let mut p = ProgressParser::new();
        let evt = p
            .parse_line("   Compiling serde v1.0.0 (/home/user/.cargo)")
            .unwrap();
        assert_eq!(evt.tool, "cargo");
        assert_eq!(evt.phase, "Compiling");
        assert_eq!(evt.target.as_deref(), Some("serde"));
    }

    #[test]
    fn parse_cargo_checking() {
        let mut p = ProgressParser::new();
        let evt = p.parse_line("   Checking crab-core v0.1.0").unwrap();
        assert_eq!(evt.tool, "cargo");
        assert_eq!(evt.phase, "Checking");
        assert_eq!(evt.target.as_deref(), Some("crab-core"));
    }

    #[test]
    fn parse_cargo_finished() {
        let mut p = ProgressParser::new();
        let evt = p
            .parse_line("    Finished dev [unoptimized + debuginfo] in 2.34s")
            .unwrap();
        assert_eq!(evt.tool, "cargo");
        assert_eq!(evt.phase, "Finished");
        assert!(evt.is_complete());
    }

    #[test]
    fn parse_cargo_tracks_count() {
        let mut p = ProgressParser::new();
        let _ = p.parse_line("   Compiling a v0.1.0");
        let _ = p.parse_line("   Compiling b v0.1.0");
        let evt = p.parse_line("   Compiling c v0.1.0").unwrap();
        // Should have incremented count to 3
        assert_eq!(evt.current, None); // no total known
        assert_eq!(evt.target.as_deref(), Some("c"));
    }

    #[test]
    fn parse_cargo_finished_resets_state() {
        let mut p = ProgressParser::new();
        let _ = p.parse_line("   Compiling a v0.1.0");
        let _ = p.parse_line("    Finished dev in 1s");
        // State should be reset, next compile starts from 1
        let evt = p.parse_line("   Compiling x v0.1.0").unwrap();
        assert_eq!(evt.target.as_deref(), Some("x"));
    }

    // ── Git parsing ──────────────────────────────────────────────────

    #[test]
    fn parse_git_cloning() {
        let mut p = ProgressParser::new();
        let evt = p.parse_line("Cloning into 'my-repo'...").unwrap();
        assert_eq!(evt.tool, "git");
        assert_eq!(evt.phase, "Cloning");
        assert_eq!(evt.target.as_deref(), Some("my-repo"));
    }

    #[test]
    fn parse_git_receiving_objects() {
        let mut p = ProgressParser::new();
        let evt = p
            .parse_line("Receiving objects:  45% (123/274)")
            .unwrap();
        assert_eq!(evt.tool, "git");
        assert_eq!(evt.phase, "Receiving objects");
        assert_eq!(evt.percent, Some(45));
        assert_eq!(evt.current, Some(123));
        assert_eq!(evt.total, Some(274));
    }

    #[test]
    fn parse_git_resolving_deltas() {
        let mut p = ProgressParser::new();
        let evt = p
            .parse_line("Resolving deltas:  100% (50/50), done.")
            .unwrap();
        assert_eq!(evt.tool, "git");
        assert_eq!(evt.phase, "Resolving deltas");
        assert!(evt.is_complete());
    }

    #[test]
    fn parse_git_enumerating() {
        let mut p = ProgressParser::new();
        let evt = p
            .parse_line("Enumerating objects: 274, done.")
            .unwrap();
        assert_eq!(evt.tool, "git");
        assert_eq!(evt.phase, "Enumerating objects");
    }

    // ── npm parsing ──────────────────────────────────────────────────

    #[test]
    fn parse_npm_installed() {
        let mut p = ProgressParser::new();
        let evt = p.parse_line("added 150 packages in 3s").unwrap();
        assert_eq!(evt.tool, "npm");
        assert_eq!(evt.phase, "installed");
        assert!(evt.is_complete());
    }

    #[test]
    fn parse_npm_warn() {
        let mut p = ProgressParser::new();
        let evt = p.parse_line("npm warn deprecated glob@7").unwrap();
        assert_eq!(evt.tool, "npm");
        assert_eq!(evt.phase, "warning");
    }

    // ── Generic percent ──────────────────────────────────────────────

    #[test]
    fn parse_generic_percent() {
        let mut p = ProgressParser::new();
        let evt = p.parse_line("Progress: 75%").unwrap();
        assert_eq!(evt.percent, Some(75));
    }

    #[test]
    fn parse_no_progress_returns_none() {
        let mut p = ProgressParser::new();
        assert!(p.parse_line("just a normal line").is_none());
        assert!(p.parse_line("").is_none());
    }

    // ── Helper functions ─────────────────────────────────────────────

    #[test]
    fn extract_percentage_basic() {
        assert_eq!(extract_percentage("45%"), Some(45));
        assert_eq!(extract_percentage("  100% done"), Some(100));
        assert_eq!(extract_percentage("no percent here"), None);
    }

    #[test]
    fn extract_percentage_clamped() {
        assert_eq!(extract_percentage("200%"), Some(100));
    }

    #[test]
    fn extract_fraction_basic() {
        assert_eq!(extract_fraction("(123/274)"), Some((123, 274)));
        assert_eq!(extract_fraction("objects: (50/100), done"), Some((50, 100)));
        assert_eq!(extract_fraction("no fraction"), None);
    }

    #[test]
    fn trimmed_after_phase_basic() {
        assert_eq!(
            trimmed_after_phase("   Compiling serde v1.0", "Compiling"),
            Some("serde v1.0")
        );
        assert_eq!(trimmed_after_phase("no match", "Compiling"), None);
    }

    // ── Parser reset ─────────────────────────────────────────────────

    #[test]
    fn parser_reset_clears_state() {
        let mut p = ProgressParser::new();
        let _ = p.parse_line("   Compiling a v0.1.0");
        p.reset();
        assert!(p.cargo_state.is_none());
    }
}
