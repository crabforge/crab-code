//! MCP server health checking, heartbeat, and auto-reconnect.
//!
//! Provides [`HealthChecker`] for periodic server health probes,
//! [`Heartbeat`] for keep-alive detection, and [`AutoReconnect`] for
//! exponential-backoff reconnection on connection loss.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, Instant};

// ─── Health status ─────────────────────────────────────────────────────

/// Health state of an MCP server connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Server is reachable and responding.
    Healthy,
    /// Server responded but slowly or with warnings.
    Degraded,
    /// Server is not reachable.
    Unreachable,
    /// Health has not been checked yet.
    Unknown,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unreachable => write!(f, "unreachable"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Result of a single health check.
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    pub server_name: String,
    pub status: HealthStatus,
    pub latency: Duration,
    pub checked_at: Instant,
    pub message: Option<String>,
}

// ─── Health checker ────────────────────────────────────────────────────

/// Configuration for the health checker.
#[derive(Debug, Clone)]
pub struct HealthCheckerConfig {
    /// How often to probe (default 30s).
    pub interval: Duration,
    /// Maximum response time before marking as degraded.
    pub degraded_threshold: Duration,
    /// Maximum response time before marking as unreachable.
    pub timeout: Duration,
    /// Number of consecutive failures before marking unreachable.
    pub failure_threshold: u32,
}

impl Default for HealthCheckerConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            degraded_threshold: Duration::from_secs(5),
            timeout: Duration::from_secs(10),
            failure_threshold: 3,
        }
    }
}

/// Tracks health state for a single MCP server.
#[derive(Debug)]
pub struct HealthChecker {
    pub server_name: String,
    config: HealthCheckerConfig,
    status: HealthStatus,
    consecutive_failures: u32,
    last_check: Option<Instant>,
    last_latency: Duration,
}

impl HealthChecker {
    #[must_use]
    pub fn new(server_name: impl Into<String>, config: HealthCheckerConfig) -> Self {
        Self {
            server_name: server_name.into(),
            config,
            status: HealthStatus::Unknown,
            consecutive_failures: 0,
            last_check: None,
            last_latency: Duration::ZERO,
        }
    }

    /// Current health status.
    #[must_use]
    pub fn status(&self) -> HealthStatus {
        self.status
    }

    /// Whether a check is due based on the configured interval.
    #[must_use]
    pub fn is_check_due(&self) -> bool {
        self.last_check
            .is_none_or(|t| t.elapsed() >= self.config.interval)
    }

    /// Record a successful health probe with the observed latency.
    pub fn record_success(&mut self, latency: Duration) -> HealthCheckResult {
        self.consecutive_failures = 0;
        self.last_check = Some(Instant::now());
        self.last_latency = latency;

        self.status = if latency >= self.config.degraded_threshold {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        HealthCheckResult {
            server_name: self.server_name.clone(),
            status: self.status,
            latency,
            checked_at: self.last_check.unwrap(),
            message: None,
        }
    }

    /// Record a failed health probe.
    pub fn record_failure(&mut self, message: impl Into<String>) -> HealthCheckResult {
        self.consecutive_failures += 1;
        self.last_check = Some(Instant::now());
        self.last_latency = self.config.timeout;

        self.status = if self.consecutive_failures >= self.config.failure_threshold {
            HealthStatus::Unreachable
        } else {
            HealthStatus::Degraded
        };

        HealthCheckResult {
            server_name: self.server_name.clone(),
            status: self.status,
            latency: self.config.timeout,
            checked_at: self.last_check.unwrap(),
            message: Some(message.into()),
        }
    }

    /// Number of consecutive failures.
    #[must_use]
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Last observed latency.
    #[must_use]
    pub fn last_latency(&self) -> Duration {
        self.last_latency
    }

    /// Access the config.
    #[must_use]
    pub fn config(&self) -> &HealthCheckerConfig {
        &self.config
    }
}

// ─── Auto-reconnect ───────────────────────────────────────────────────

/// Configuration for exponential backoff reconnection.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Initial delay before first retry.
    pub initial_delay: Duration,
    /// Maximum delay cap.
    pub max_delay: Duration,
    /// Multiplier per attempt (default 2.0).
    pub multiplier: f64,
    /// Maximum number of attempts (0 = unlimited).
    pub max_attempts: u32,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: 10,
        }
    }
}

/// State of the reconnection attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectState {
    /// Connected and healthy — no reconnection needed.
    Connected,
    /// Attempting to reconnect.
    Reconnecting,
    /// All attempts exhausted.
    GivenUp,
}

impl fmt::Display for ReconnectState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => write!(f, "connected"),
            Self::Reconnecting => write!(f, "reconnecting"),
            Self::GivenUp => write!(f, "given_up"),
        }
    }
}

/// Manages exponential-backoff reconnection for an MCP server.
#[derive(Debug)]
pub struct AutoReconnect {
    pub server_name: String,
    config: ReconnectConfig,
    state: ReconnectState,
    attempt: u32,
    next_delay: Duration,
}

impl AutoReconnect {
    #[must_use]
    pub fn new(server_name: impl Into<String>, config: ReconnectConfig) -> Self {
        Self {
            server_name: server_name.into(),
            state: ReconnectState::Connected,
            attempt: 0,
            next_delay: config.initial_delay,
            config,
        }
    }

    /// Current reconnection state.
    #[must_use]
    pub fn state(&self) -> ReconnectState {
        self.state
    }

    /// Current attempt number (0 = no attempts yet).
    #[must_use]
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Signal that the connection was lost. Returns the delay before next retry,
    /// or `None` if max attempts exhausted.
    #[must_use]
    pub fn connection_lost(&mut self) -> Option<Duration> {
        if self.config.max_attempts > 0 && self.attempt >= self.config.max_attempts {
            self.state = ReconnectState::GivenUp;
            return None;
        }

        self.state = ReconnectState::Reconnecting;
        self.attempt += 1;

        let delay = self.next_delay;
        // Advance delay with exponential backoff, capped at max
        let next = Duration::from_secs_f64(delay.as_secs_f64() * self.config.multiplier);
        self.next_delay = if next > self.config.max_delay {
            self.config.max_delay
        } else {
            next
        };

        Some(delay)
    }

    /// Signal that the connection has been re-established. Resets attempt count.
    pub fn connection_restored(&mut self) {
        self.state = ReconnectState::Connected;
        self.attempt = 0;
        self.next_delay = self.config.initial_delay;
    }

    /// Reset to initial state.
    pub fn reset(&mut self) {
        self.connection_restored();
    }
}

// ─── Heartbeat ─────────────────────────────────────────────────────────

/// Tracks heartbeat timing for a connection.
#[derive(Debug)]
pub struct Heartbeat {
    /// How often to send a heartbeat.
    pub interval: Duration,
    /// How long without a response before the connection is considered dead.
    pub timeout: Duration,
    last_sent: Option<Instant>,
    last_received: Option<Instant>,
    missed_count: u32,
}

impl Heartbeat {
    #[must_use]
    pub fn new(interval: Duration, timeout: Duration) -> Self {
        Self {
            interval,
            timeout,
            last_sent: None,
            last_received: None,
            missed_count: 0,
        }
    }

    /// Create with sensible defaults (10s interval, 30s timeout).
    #[must_use]
    pub fn default_config() -> Self {
        Self::new(Duration::from_secs(10), Duration::from_secs(30))
    }

    /// Whether it's time to send a heartbeat.
    #[must_use]
    pub fn should_send(&self) -> bool {
        self.last_sent
            .is_none_or(|t| t.elapsed() >= self.interval)
    }

    /// Record that a heartbeat was sent.
    pub fn mark_sent(&mut self) {
        self.last_sent = Some(Instant::now());
    }

    /// Record that a heartbeat response was received.
    pub fn mark_received(&mut self) {
        self.last_received = Some(Instant::now());
        self.missed_count = 0;
    }

    /// Record a missed heartbeat (sent but no response within timeout).
    pub fn mark_missed(&mut self) {
        self.missed_count += 1;
    }

    /// Whether the connection appears dead (no response within timeout).
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.last_sent.is_some_and(|sent| {
            let no_response = self
                .last_received
                .is_none_or(|recv| recv < sent);
            no_response && sent.elapsed() >= self.timeout
        })
    }

    /// Number of consecutive missed heartbeats.
    #[must_use]
    pub fn missed_count(&self) -> u32 {
        self.missed_count
    }

    /// Time since last received heartbeat response, or `None` if never received.
    #[must_use]
    pub fn time_since_last_response(&self) -> Option<Duration> {
        self.last_received.map(|t| t.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HealthStatus tests ──

    #[test]
    fn health_status_display() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(HealthStatus::Unreachable.to_string(), "unreachable");
        assert_eq!(HealthStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn health_status_serde_roundtrip() {
        for status in [
            HealthStatus::Healthy,
            HealthStatus::Degraded,
            HealthStatus::Unreachable,
            HealthStatus::Unknown,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: HealthStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    // ── HealthChecker tests ──

    #[test]
    fn checker_initial_state() {
        let hc = HealthChecker::new("test-server", HealthCheckerConfig::default());
        assert_eq!(hc.status(), HealthStatus::Unknown);
        assert_eq!(hc.consecutive_failures(), 0);
        assert!(hc.is_check_due());
    }

    #[test]
    fn checker_record_success_healthy() {
        let mut hc = HealthChecker::new("s", HealthCheckerConfig::default());
        let result = hc.record_success(Duration::from_millis(100));
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(hc.status(), HealthStatus::Healthy);
        assert_eq!(hc.consecutive_failures(), 0);
    }

    #[test]
    fn checker_record_success_degraded() {
        let mut hc = HealthChecker::new("s", HealthCheckerConfig {
            degraded_threshold: Duration::from_secs(1),
            ..Default::default()
        });
        let result = hc.record_success(Duration::from_secs(2));
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn checker_record_failure_below_threshold() {
        let mut hc = HealthChecker::new("s", HealthCheckerConfig {
            failure_threshold: 3,
            ..Default::default()
        });
        let r = hc.record_failure("timeout");
        assert_eq!(r.status, HealthStatus::Degraded);
        assert_eq!(hc.consecutive_failures(), 1);
    }

    #[test]
    fn checker_record_failure_at_threshold() {
        let mut hc = HealthChecker::new("s", HealthCheckerConfig {
            failure_threshold: 2,
            ..Default::default()
        });
        hc.record_failure("fail 1");
        let r = hc.record_failure("fail 2");
        assert_eq!(r.status, HealthStatus::Unreachable);
        assert_eq!(hc.consecutive_failures(), 2);
    }

    #[test]
    fn checker_success_resets_failures() {
        let mut hc = HealthChecker::new("s", HealthCheckerConfig::default());
        hc.record_failure("f1");
        hc.record_failure("f2");
        assert_eq!(hc.consecutive_failures(), 2);
        hc.record_success(Duration::from_millis(50));
        assert_eq!(hc.consecutive_failures(), 0);
        assert_eq!(hc.status(), HealthStatus::Healthy);
    }

    #[test]
    fn checker_is_check_due_after_check() {
        let mut hc = HealthChecker::new("s", HealthCheckerConfig {
            interval: Duration::from_secs(3600), // 1 hour
            ..Default::default()
        });
        assert!(hc.is_check_due());
        hc.record_success(Duration::from_millis(10));
        assert!(!hc.is_check_due()); // Just checked, not due yet
    }

    // ── AutoReconnect tests ──

    #[test]
    fn reconnect_initial_state() {
        let ar = AutoReconnect::new("s", ReconnectConfig::default());
        assert_eq!(ar.state(), ReconnectState::Connected);
        assert_eq!(ar.attempt(), 0);
    }

    #[test]
    fn reconnect_exponential_backoff() {
        let mut ar = AutoReconnect::new("s", ReconnectConfig {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: 5,
        });

        let d1 = ar.connection_lost().unwrap();
        assert_eq!(d1, Duration::from_secs(1));
        assert_eq!(ar.attempt(), 1);

        let d2 = ar.connection_lost().unwrap();
        assert_eq!(d2, Duration::from_secs(2));

        let d3 = ar.connection_lost().unwrap();
        assert_eq!(d3, Duration::from_secs(4));
    }

    #[test]
    fn reconnect_delay_capped_at_max() {
        let mut ar = AutoReconnect::new("s", ReconnectConfig {
            initial_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: 10,
        });

        let _ = ar.connection_lost(); // 30s
        let d = ar.connection_lost().unwrap(); // 60s (capped)
        assert_eq!(d, Duration::from_secs(60));

        let d = ar.connection_lost().unwrap(); // still 60s
        assert_eq!(d, Duration::from_secs(60));
    }

    #[test]
    fn reconnect_gives_up_after_max_attempts() {
        let mut ar = AutoReconnect::new("s", ReconnectConfig {
            max_attempts: 2,
            ..Default::default()
        });

        assert!(ar.connection_lost().is_some()); // attempt 1
        assert!(ar.connection_lost().is_some()); // attempt 2
        assert!(ar.connection_lost().is_none()); // exhausted
        assert_eq!(ar.state(), ReconnectState::GivenUp);
    }

    #[test]
    fn reconnect_connection_restored_resets() {
        let mut ar = AutoReconnect::new("s", ReconnectConfig::default());
        let _ = ar.connection_lost();
        let _ = ar.connection_lost();
        assert_eq!(ar.attempt(), 2);
        assert_eq!(ar.state(), ReconnectState::Reconnecting);

        ar.connection_restored();
        assert_eq!(ar.state(), ReconnectState::Connected);
        assert_eq!(ar.attempt(), 0);
    }

    #[test]
    fn reconnect_state_display() {
        assert_eq!(ReconnectState::Connected.to_string(), "connected");
        assert_eq!(ReconnectState::Reconnecting.to_string(), "reconnecting");
        assert_eq!(ReconnectState::GivenUp.to_string(), "given_up");
    }

    // ── Heartbeat tests ──

    #[test]
    fn heartbeat_initial_state() {
        let hb = Heartbeat::default_config();
        assert!(hb.should_send());
        assert!(!hb.is_dead());
        assert_eq!(hb.missed_count(), 0);
        assert!(hb.time_since_last_response().is_none());
    }

    #[test]
    fn heartbeat_mark_sent_and_received() {
        let mut hb = Heartbeat::new(Duration::from_secs(10), Duration::from_secs(30));
        hb.mark_sent();
        assert!(!hb.should_send()); // Just sent
        hb.mark_received();
        assert!(!hb.is_dead());
        assert!(hb.time_since_last_response().is_some());
    }

    #[test]
    fn heartbeat_missed_count() {
        let mut hb = Heartbeat::default_config();
        hb.mark_missed();
        hb.mark_missed();
        assert_eq!(hb.missed_count(), 2);
        hb.mark_received();
        assert_eq!(hb.missed_count(), 0);
    }

    #[test]
    fn heartbeat_dead_detection() {
        let mut hb = Heartbeat::new(
            Duration::from_millis(10),
            Duration::from_millis(1), // Very short timeout for testing
        );
        hb.mark_sent();
        // Sleep a tiny bit to exceed timeout
        std::thread::sleep(Duration::from_millis(5));
        assert!(hb.is_dead());
    }

    #[test]
    fn heartbeat_not_dead_after_response() {
        let mut hb = Heartbeat::new(
            Duration::from_millis(10),
            Duration::from_millis(1),
        );
        hb.mark_sent();
        hb.mark_received(); // Response came in
        // Even after timeout, last_received >= last_sent
        std::thread::sleep(Duration::from_millis(5));
        assert!(!hb.is_dead());
    }

    #[test]
    fn heartbeat_should_send_after_interval() {
        let mut hb = Heartbeat::new(
            Duration::from_millis(1), // Very short interval
            Duration::from_secs(30),
        );
        hb.mark_sent();
        std::thread::sleep(Duration::from_millis(5));
        assert!(hb.should_send());
    }
}
