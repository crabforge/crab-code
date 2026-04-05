//! Colored terminal output and progress indicators.
//!
//! Provides styled printing helpers for consistent CLI output, plus a
//! spinner for long-running operations (e.g., waiting for LLM responses).

use owo_colors::OwoColorize;
use std::fmt;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ─── Styled output helpers ─────────────────────────────────────

/// Print an info-level message with a blue prefix.
#[allow(dead_code)]
pub fn info(msg: &str) {
    eprintln!("{} {msg}", "info:".blue().bold());
}

/// Print a success message with a green prefix.
#[allow(dead_code)]
pub fn success(msg: &str) {
    eprintln!("{} {msg}", "ok:".green().bold());
}

/// Print a warning with a yellow prefix.
#[allow(dead_code)]
pub fn warn(msg: &str) {
    eprintln!("{} {msg}", "warn:".yellow().bold());
}

/// Print an error with a red prefix.
pub fn error(msg: &str) {
    eprintln!("{} {msg}", "error:".red().bold());
}

/// Print a tool-use header with a cyan prefix.
pub fn tool_use(name: &str) {
    eprintln!("{} {}", "tool:".cyan().bold(), name.cyan());
}

/// Print a tool result (truncated if too long).
pub fn tool_result(text: &str, is_error: bool) {
    if is_error {
        eprintln!("{} {text}", "tool error:".red().bold());
    } else {
        let prefix = "result:".dimmed();
        let display = if text.len() > 500 {
            format!("{}...", &text[..500])
        } else {
            text.to_string()
        };
        eprintln!("{prefix} {display}");
    }
}

/// Print the startup banner with version info.
pub fn banner(version: &str, provider: &str, model: &str, permission_mode: &impl fmt::Display) {
    eprintln!(
        "{} {} {} provider={} model={} permissions={}",
        "crab-code".green().bold(),
        version.dimmed(),
        "|".dimmed(),
        provider.cyan(),
        model.cyan(),
        format!("{permission_mode}").yellow(),
    );
}

/// Print a compact header.
pub fn compact_start(strategy: &str) {
    eprintln!(
        "{} Starting compaction: {strategy}",
        "compact:".magenta().bold()
    );
}

/// Print a compact result.
pub fn compact_end(removed: usize, tokens: u64) {
    eprintln!(
        "{} removed {removed} messages, now {tokens} tokens",
        "compact:".magenta().bold()
    );
}

/// Print token usage warning.
pub fn token_warning(pct: f32, used: u64, limit: u64) {
    eprintln!(
        "{} Token usage {:.0}% ({used}/{limit})",
        "warn:".yellow().bold(),
        pct * 100.0,
    );
}

// ─── Spinner ───────────────────────────────────────────────────

/// A simple terminal spinner that runs in a background thread.
///
/// Call [`Spinner::start`] to begin, and [`Spinner::stop`] (or drop) to halt.
pub struct Spinner {
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Spinner {
    /// Start a spinner with the given message.
    pub fn start(message: &str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let msg = message.to_string();

        let handle = std::thread::spawn(move || {
            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut i = 0;
            while running_clone.load(Ordering::Relaxed) {
                eprint!(
                    "\r{} {}",
                    frames[i % frames.len()].to_string().cyan(),
                    msg.dimmed()
                );
                let _ = std::io::stderr().flush();
                std::thread::sleep(std::time::Duration::from_millis(80));
                i += 1;
            }
            // Clear the spinner line
            eprint!("\r{}\r", " ".repeat(msg.len() + 4));
            let _ = std::io::stderr().flush();
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    /// Stop the spinner.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_starts_and_stops() {
        let mut spinner = Spinner::start("thinking...");
        std::thread::sleep(std::time::Duration::from_millis(200));
        spinner.stop();
        // Should not panic or hang
        assert!(!spinner.running.load(Ordering::Relaxed));
    }

    #[test]
    fn spinner_stops_on_drop() {
        let running;
        {
            let spinner = Spinner::start("dropping...");
            running = Arc::clone(&spinner.running);
            assert!(running.load(Ordering::Relaxed));
        }
        // After drop, spinner should have stopped
        // Give the thread a moment to fully stop
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!running.load(Ordering::Relaxed));
    }

    #[test]
    fn double_stop_is_safe() {
        let mut spinner = Spinner::start("double stop");
        spinner.stop();
        spinner.stop(); // Should not panic
    }

    // Test that styled output functions don't panic
    // (they write to stderr, which is fine in tests)

    #[test]
    fn info_does_not_panic() {
        info("test message");
    }

    #[test]
    fn success_does_not_panic() {
        success("it worked");
    }

    #[test]
    fn warn_does_not_panic() {
        warn("be careful");
    }

    #[test]
    fn error_does_not_panic() {
        error("something broke");
    }

    #[test]
    fn tool_use_does_not_panic() {
        tool_use("bash");
    }

    #[test]
    fn tool_result_normal() {
        tool_result("some output", false);
    }

    #[test]
    fn tool_result_error() {
        tool_result("failed", true);
    }

    #[test]
    fn tool_result_truncation() {
        let long = "x".repeat(1000);
        tool_result(&long, false);
    }

    #[test]
    fn banner_does_not_panic() {
        banner("0.1.0", "anthropic", "claude-sonnet-4-20250514", &"default");
    }

    #[test]
    fn compact_helpers_do_not_panic() {
        compact_start("sliding-window");
        compact_end(5, 10000);
    }

    #[test]
    fn token_warning_does_not_panic() {
        token_warning(0.85_f32, 170_000, 200_000);
    }
}
