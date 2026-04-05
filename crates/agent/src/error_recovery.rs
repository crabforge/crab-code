//! Error recovery: classification, strategy selection, circuit breaker, and
//! graceful degradation for resilient agent operation.
//!
//! Builds on the retry module to provide higher-level error handling that
//! adapts behavior based on error type and failure patterns.

use std::collections::HashMap;
use std::time::{Duration, Instant};

// ── Error classification ──────────────────────────────────────────────

/// Classified error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// Temporary failure likely to succeed on retry (network hiccup, 503).
    Transient,
    /// Permanent failure — retrying won't help (invalid input, 404).
    Permanent,
    /// Rate limit exceeded — retry after backoff (429).
    RateLimit,
    /// Authentication/authorization failure (401, 403).
    Auth,
    /// Request timed out — may succeed with longer timeout or retry.
    Timeout,
    /// Unknown error category.
    Unknown,
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient => write!(f, "transient"),
            Self::Permanent => write!(f, "permanent"),
            Self::RateLimit => write!(f, "rate_limit"),
            Self::Auth => write!(f, "auth"),
            Self::Timeout => write!(f, "timeout"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Classifies errors into categories based on error message patterns and
/// HTTP status codes.
pub struct ErrorClassifier;

impl ErrorClassifier {
    /// Classify an error from its message string.
    #[must_use]
    pub fn classify(error_msg: &str) -> ErrorCategory {
        let lower = error_msg.to_lowercase();

        // Rate limit patterns
        if lower.contains("rate limit")
            || lower.contains("429")
            || lower.contains("too many requests")
            || lower.contains("quota exceeded")
        {
            return ErrorCategory::RateLimit;
        }

        // Auth patterns
        if lower.contains("401")
            || lower.contains("403")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("invalid api key")
            || lower.contains("authentication")
            || lower.contains("permission denied")
        {
            return ErrorCategory::Auth;
        }

        // Timeout patterns
        if lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("deadline exceeded")
            || lower.contains("request took too long")
        {
            return ErrorCategory::Timeout;
        }

        // Permanent patterns
        if lower.contains("404")
            || lower.contains("not found")
            || lower.contains("invalid")
            || lower.contains("malformed")
            || lower.contains("bad request")
            || lower.contains("400")
            || lower.contains("unsupported")
            || lower.contains("unprocessable")
            || lower.contains("422")
        {
            return ErrorCategory::Permanent;
        }

        // Transient patterns
        if lower.contains("500")
            || lower.contains("502")
            || lower.contains("503")
            || lower.contains("504")
            || lower.contains("internal server error")
            || lower.contains("service unavailable")
            || lower.contains("bad gateway")
            || lower.contains("connection refused")
            || lower.contains("connection reset")
            || lower.contains("broken pipe")
            || lower.contains("temporarily")
        {
            return ErrorCategory::Transient;
        }

        ErrorCategory::Unknown
    }

    /// Classify from an HTTP status code.
    #[must_use]
    pub fn classify_status(status: u16) -> ErrorCategory {
        match status {
            429 => ErrorCategory::RateLimit,
            401 | 403 => ErrorCategory::Auth,
            408 | 504 => ErrorCategory::Timeout,
            400 | 404 | 405 | 422 | 200..=299 => ErrorCategory::Permanent,
            500 | 502 | 503 => ErrorCategory::Transient,
            _ => ErrorCategory::Unknown,
        }
    }
}

// ── Recovery strategy ─────────────────────────────────────────────────

/// Action to take when recovering from an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Retry the operation after a delay.
    Retry { delay: Duration, max_attempts: u32 },
    /// Fall back to an alternative (e.g., different model, simpler approach).
    Fallback { reason: String },
    /// Abort the operation — error is not recoverable.
    Abort { reason: String },
    /// Ask the user for guidance.
    AskUser { message: String },
}

impl std::fmt::Display for RecoveryAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retry {
                delay,
                max_attempts,
            } => {
                write!(
                    f,
                    "retry (delay: {}ms, max: {})",
                    delay.as_millis(),
                    max_attempts
                )
            }
            Self::Fallback { reason } => write!(f, "fallback: {reason}"),
            Self::Abort { reason } => write!(f, "abort: {reason}"),
            Self::AskUser { message } => write!(f, "ask user: {message}"),
        }
    }
}

/// Selects recovery strategies based on error category and context.
#[derive(Debug, Clone)]
pub struct RecoveryStrategy {
    /// Maximum retry attempts for transient errors.
    pub transient_max_retries: u32,
    /// Base delay for transient retries.
    pub transient_base_delay: Duration,
    /// Delay for rate limit retries.
    pub rate_limit_delay: Duration,
    /// Maximum retry attempts for rate limits.
    pub rate_limit_max_retries: u32,
    /// Maximum retry attempts for timeouts.
    pub timeout_max_retries: u32,
    /// Delay for timeout retries.
    pub timeout_delay: Duration,
}

impl Default for RecoveryStrategy {
    fn default() -> Self {
        Self {
            transient_max_retries: 3,
            transient_base_delay: Duration::from_secs(1),
            rate_limit_delay: Duration::from_secs(10),
            rate_limit_max_retries: 5,
            timeout_max_retries: 2,
            timeout_delay: Duration::from_secs(5),
        }
    }
}

impl RecoveryStrategy {
    /// Determine the recovery action for a given error category.
    #[must_use]
    pub fn recommend(&self, category: ErrorCategory) -> RecoveryAction {
        match category {
            ErrorCategory::Transient => RecoveryAction::Retry {
                delay: self.transient_base_delay,
                max_attempts: self.transient_max_retries,
            },
            ErrorCategory::RateLimit => RecoveryAction::Retry {
                delay: self.rate_limit_delay,
                max_attempts: self.rate_limit_max_retries,
            },
            ErrorCategory::Timeout => RecoveryAction::Retry {
                delay: self.timeout_delay,
                max_attempts: self.timeout_max_retries,
            },
            ErrorCategory::Auth => RecoveryAction::AskUser {
                message: "Authentication failed. Please check your API key or credentials.".into(),
            },
            ErrorCategory::Permanent => RecoveryAction::Abort {
                reason: "The request is invalid and cannot be retried.".into(),
            },
            ErrorCategory::Unknown => RecoveryAction::Fallback {
                reason: "Unknown error — attempting alternative approach.".into(),
            },
        }
    }

    /// Determine recovery action with attempt context.
    ///
    /// If the maximum retries for a category are exhausted, escalates to
    /// fallback or abort.
    #[must_use]
    pub fn recommend_with_attempts(
        &self,
        category: ErrorCategory,
        attempts_so_far: u32,
    ) -> RecoveryAction {
        let base = self.recommend(category);
        match &base {
            RecoveryAction::Retry { max_attempts, .. } => {
                if attempts_so_far >= *max_attempts {
                    // Escalate: retries exhausted
                    match category {
                        ErrorCategory::RateLimit => RecoveryAction::AskUser {
                            message: "Rate limit persists after retries. Wait or check quota."
                                .into(),
                        },
                        ErrorCategory::Timeout => RecoveryAction::Fallback {
                            reason: "Timeout persists — trying simpler request.".into(),
                        },
                        _ => RecoveryAction::Abort {
                            reason: format!("Retries exhausted after {attempts_so_far} attempts."),
                        },
                    }
                } else {
                    base
                }
            }
            _ => base,
        }
    }
}

// ── Circuit breaker ───────────────────────────────────────────────────

/// State of a circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Tripped — requests are blocked.
    Open,
    /// Partially open — allowing a single probe request.
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// Duration to keep the circuit open before allowing a probe.
    pub open_duration: Duration,
    /// Number of successes in half-open state to close the circuit.
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            open_duration: Duration::from_secs(30),
            success_threshold: 2,
        }
    }
}

/// Circuit breaker: trips after consecutive failures, blocks requests
/// until a cooldown period passes, then allows probe requests.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: CircuitState,
    consecutive_failures: u32,
    half_open_successes: u32,
    last_failure_time: Option<Instant>,
    total_trips: u32,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with default config.
    #[must_use]
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: CircuitState::Closed,
            consecutive_failures: 0,
            half_open_successes: 0,
            last_failure_time: None,
            total_trips: 0,
        }
    }

    /// Check if a request should be allowed.
    ///
    /// Returns `true` if the circuit allows the request.
    /// Transitions from Open to `HalfOpen` if the cooldown has elapsed.
    pub fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => {
                if let Some(last_fail) = self.last_failure_time {
                    if last_fail.elapsed() >= self.config.open_duration {
                        self.state = CircuitState::HalfOpen;
                        self.half_open_successes = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    // No failure recorded — shouldn't be open, reset
                    self.state = CircuitState::Closed;
                    true
                }
            }
        }
    }

    /// Record a successful request.
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.consecutive_failures = 0;
            }
            CircuitState::HalfOpen => {
                self.half_open_successes += 1;
                if self.half_open_successes >= self.config.success_threshold {
                    self.state = CircuitState::Closed;
                    self.consecutive_failures = 0;
                }
            }
            CircuitState::Open => {
                // Shouldn't happen if allow_request is checked first
            }
        }
    }

    /// Record a failed request.
    pub fn record_failure(&mut self) {
        self.last_failure_time = Some(Instant::now());
        match self.state {
            CircuitState::Closed => {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= self.config.failure_threshold {
                    self.state = CircuitState::Open;
                    self.total_trips += 1;
                }
            }
            CircuitState::HalfOpen => {
                // Probe failed — reopen
                self.state = CircuitState::Open;
                self.total_trips += 1;
            }
            CircuitState::Open => {
                // Already open
            }
        }
    }

    /// Get the current circuit state.
    #[must_use]
    pub fn state(&self) -> CircuitState {
        self.state
    }

    /// Get the number of consecutive failures.
    #[must_use]
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Get the total number of times the circuit has tripped.
    #[must_use]
    pub fn total_trips(&self) -> u32 {
        self.total_trips
    }

    /// Manually reset the circuit to closed state.
    pub fn reset(&mut self) {
        self.state = CircuitState::Closed;
        self.consecutive_failures = 0;
        self.half_open_successes = 0;
        self.last_failure_time = None;
    }
}

// ── Graceful degradation ──────────────────────────────────────────────

/// Feature that can be degraded (disabled) under error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DegradableFeature {
    /// Context window auto-injection of relevant files.
    SmartContext,
    /// Memory retrieval and injection.
    MemoryRetrieval,
    /// Tool execution (fall back to text-only responses).
    ToolExecution,
    /// Streaming output (fall back to batch).
    Streaming,
    /// Multi-agent coordination.
    MultiAgent,
    /// Code navigation features.
    CodeNavigation,
}

impl std::fmt::Display for DegradableFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SmartContext => write!(f, "smart_context"),
            Self::MemoryRetrieval => write!(f, "memory_retrieval"),
            Self::ToolExecution => write!(f, "tool_execution"),
            Self::Streaming => write!(f, "streaming"),
            Self::MultiAgent => write!(f, "multi_agent"),
            Self::CodeNavigation => write!(f, "code_navigation"),
        }
    }
}

/// Priority level for a degradable feature (lower = shed first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeaturePriority(pub u8);

impl FeaturePriority {
    /// Core features that should almost never be disabled.
    pub const CORE: Self = Self(100);
    /// Important but not critical.
    pub const HIGH: Self = Self(75);
    /// Nice-to-have features.
    pub const MEDIUM: Self = Self(50);
    /// Optional enhancements.
    pub const LOW: Self = Self(25);
}

/// Manages graceful degradation: disables non-essential features when
/// errors accumulate to maintain basic functionality.
#[derive(Debug)]
pub struct GracefulDegradation {
    /// Feature -> (priority, enabled).
    features: HashMap<DegradableFeature, (FeaturePriority, bool)>,
    /// Current degradation level (0 = normal, higher = more degraded).
    degradation_level: u8,
    /// Maximum degradation level.
    max_level: u8,
}

impl GracefulDegradation {
    /// Create with default feature set and priorities.
    #[must_use]
    pub fn new() -> Self {
        let mut features = HashMap::new();
        features.insert(
            DegradableFeature::SmartContext,
            (FeaturePriority::LOW, true),
        );
        features.insert(
            DegradableFeature::MemoryRetrieval,
            (FeaturePriority::MEDIUM, true),
        );
        features.insert(
            DegradableFeature::CodeNavigation,
            (FeaturePriority::LOW, true),
        );
        features.insert(DegradableFeature::Streaming, (FeaturePriority::HIGH, true));
        features.insert(
            DegradableFeature::MultiAgent,
            (FeaturePriority::MEDIUM, true),
        );
        features.insert(
            DegradableFeature::ToolExecution,
            (FeaturePriority::CORE, true),
        );

        Self {
            features,
            degradation_level: 0,
            max_level: 4,
        }
    }

    /// Check if a feature is currently enabled.
    #[must_use]
    pub fn is_enabled(&self, feature: DegradableFeature) -> bool {
        self.features
            .get(&feature)
            .is_some_and(|(_, enabled)| *enabled)
    }

    /// Increase degradation level, disabling lowest-priority features first.
    ///
    /// Returns the list of features that were disabled in this step.
    pub fn degrade(&mut self) -> Vec<DegradableFeature> {
        if self.degradation_level >= self.max_level {
            return Vec::new();
        }

        self.degradation_level += 1;
        let threshold = self.priority_threshold();

        let mut disabled = Vec::new();
        for (feature, (priority, enabled)) in &mut self.features {
            if *enabled && priority.0 < threshold {
                *enabled = false;
                disabled.push(*feature);
            }
        }

        disabled
    }

    /// Decrease degradation level, re-enabling features.
    ///
    /// Returns the list of features that were re-enabled.
    pub fn recover(&mut self) -> Vec<DegradableFeature> {
        if self.degradation_level == 0 {
            return Vec::new();
        }

        self.degradation_level -= 1;
        let threshold = self.priority_threshold();

        let mut enabled = Vec::new();
        for (feature, (priority, is_enabled)) in &mut self.features {
            if !*is_enabled && priority.0 >= threshold {
                *is_enabled = true;
                enabled.push(*feature);
            }
        }

        enabled
    }

    /// Reset to full functionality.
    pub fn reset(&mut self) {
        self.degradation_level = 0;
        for (_, enabled) in self.features.values_mut() {
            *enabled = true;
        }
    }

    /// Get the current degradation level (0 = normal).
    #[must_use]
    pub fn level(&self) -> u8 {
        self.degradation_level
    }

    /// Get the list of currently disabled features.
    #[must_use]
    pub fn disabled_features(&self) -> Vec<DegradableFeature> {
        self.features
            .iter()
            .filter(|(_, (_, enabled))| !enabled)
            .map(|(f, _)| *f)
            .collect()
    }

    /// Get the list of currently enabled features.
    #[must_use]
    pub fn enabled_features(&self) -> Vec<DegradableFeature> {
        self.features
            .iter()
            .filter(|(_, (_, enabled))| *enabled)
            .map(|(f, _)| *f)
            .collect()
    }

    /// Manually disable a specific feature.
    pub fn disable_feature(&mut self, feature: DegradableFeature) {
        if let Some((_, enabled)) = self.features.get_mut(&feature) {
            *enabled = false;
        }
    }

    /// Manually enable a specific feature.
    pub fn enable_feature(&mut self, feature: DegradableFeature) {
        if let Some((_, enabled)) = self.features.get_mut(&feature) {
            *enabled = true;
        }
    }

    /// Compute the priority threshold for the current degradation level.
    /// Features with priority below this threshold are disabled.
    fn priority_threshold(&self) -> u8 {
        // Level 0: threshold 0 (nothing disabled)
        // Level 1: threshold 30 (LOW disabled)
        // Level 2: threshold 55 (LOW + MEDIUM disabled)
        // Level 3: threshold 80 (LOW + MEDIUM + HIGH disabled)
        // Level 4: threshold 101 (everything disabled)
        match self.degradation_level {
            0 => 0,
            1 => 30,
            2 => 55,
            3 => 80,
            _ => 101,
        }
    }
}

impl Default for GracefulDegradation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ErrorCategory ──────────────────────────────────────────────

    #[test]
    fn error_category_display() {
        assert_eq!(ErrorCategory::Transient.to_string(), "transient");
        assert_eq!(ErrorCategory::Permanent.to_string(), "permanent");
        assert_eq!(ErrorCategory::RateLimit.to_string(), "rate_limit");
        assert_eq!(ErrorCategory::Auth.to_string(), "auth");
        assert_eq!(ErrorCategory::Timeout.to_string(), "timeout");
        assert_eq!(ErrorCategory::Unknown.to_string(), "unknown");
    }

    // ── ErrorClassifier ────────────────────────────────────────────

    #[test]
    fn classify_rate_limit() {
        assert_eq!(
            ErrorClassifier::classify("Rate limit exceeded"),
            ErrorCategory::RateLimit
        );
        assert_eq!(
            ErrorClassifier::classify("HTTP 429 Too Many Requests"),
            ErrorCategory::RateLimit
        );
        assert_eq!(
            ErrorClassifier::classify("Quota exceeded for model"),
            ErrorCategory::RateLimit
        );
    }

    #[test]
    fn classify_auth() {
        assert_eq!(
            ErrorClassifier::classify("401 Unauthorized"),
            ErrorCategory::Auth
        );
        assert_eq!(
            ErrorClassifier::classify("403 Forbidden"),
            ErrorCategory::Auth
        );
        assert_eq!(
            ErrorClassifier::classify("Invalid API key provided"),
            ErrorCategory::Auth
        );
        assert_eq!(
            ErrorClassifier::classify("Permission denied"),
            ErrorCategory::Auth
        );
    }

    #[test]
    fn classify_timeout() {
        assert_eq!(
            ErrorClassifier::classify("Request timeout"),
            ErrorCategory::Timeout
        );
        assert_eq!(
            ErrorClassifier::classify("Connection timed out"),
            ErrorCategory::Timeout
        );
        assert_eq!(
            ErrorClassifier::classify("Deadline exceeded"),
            ErrorCategory::Timeout
        );
    }

    #[test]
    fn classify_permanent() {
        assert_eq!(
            ErrorClassifier::classify("404 Not Found"),
            ErrorCategory::Permanent
        );
        assert_eq!(
            ErrorClassifier::classify("Invalid request body"),
            ErrorCategory::Permanent
        );
        assert_eq!(
            ErrorClassifier::classify("400 Bad Request"),
            ErrorCategory::Permanent
        );
        assert_eq!(
            ErrorClassifier::classify("Malformed JSON input"),
            ErrorCategory::Permanent
        );
    }

    #[test]
    fn classify_transient() {
        assert_eq!(
            ErrorClassifier::classify("500 Internal Server Error"),
            ErrorCategory::Transient
        );
        assert_eq!(
            ErrorClassifier::classify("503 Service Unavailable"),
            ErrorCategory::Transient
        );
        assert_eq!(
            ErrorClassifier::classify("Connection refused"),
            ErrorCategory::Transient
        );
        assert_eq!(
            ErrorClassifier::classify("Connection reset by peer"),
            ErrorCategory::Transient
        );
    }

    #[test]
    fn classify_unknown() {
        assert_eq!(
            ErrorClassifier::classify("Something went wrong"),
            ErrorCategory::Unknown
        );
        assert_eq!(ErrorClassifier::classify(""), ErrorCategory::Unknown);
    }

    #[test]
    fn classify_status_codes() {
        assert_eq!(
            ErrorClassifier::classify_status(429),
            ErrorCategory::RateLimit
        );
        assert_eq!(ErrorClassifier::classify_status(401), ErrorCategory::Auth);
        assert_eq!(ErrorClassifier::classify_status(403), ErrorCategory::Auth);
        assert_eq!(
            ErrorClassifier::classify_status(408),
            ErrorCategory::Timeout
        );
        assert_eq!(
            ErrorClassifier::classify_status(504),
            ErrorCategory::Timeout
        );
        assert_eq!(
            ErrorClassifier::classify_status(400),
            ErrorCategory::Permanent
        );
        assert_eq!(
            ErrorClassifier::classify_status(404),
            ErrorCategory::Permanent
        );
        assert_eq!(
            ErrorClassifier::classify_status(500),
            ErrorCategory::Transient
        );
        assert_eq!(
            ErrorClassifier::classify_status(503),
            ErrorCategory::Transient
        );
        assert_eq!(
            ErrorClassifier::classify_status(418),
            ErrorCategory::Unknown
        );
    }

    // ── RecoveryAction ─────────────────────────────────────────────

    #[test]
    fn recovery_action_display() {
        let retry = RecoveryAction::Retry {
            delay: Duration::from_secs(1),
            max_attempts: 3,
        };
        assert!(retry.to_string().contains("retry"));

        let fallback = RecoveryAction::Fallback {
            reason: "test".into(),
        };
        assert!(fallback.to_string().contains("fallback"));

        let abort = RecoveryAction::Abort {
            reason: "fatal".into(),
        };
        assert!(abort.to_string().contains("abort"));

        let ask = RecoveryAction::AskUser {
            message: "help".into(),
        };
        assert!(ask.to_string().contains("ask user"));
    }

    // ── RecoveryStrategy ───────────────────────────────────────────

    #[test]
    fn strategy_defaults() {
        let s = RecoveryStrategy::default();
        assert_eq!(s.transient_max_retries, 3);
        assert_eq!(s.rate_limit_max_retries, 5);
        assert_eq!(s.timeout_max_retries, 2);
    }

    #[test]
    fn strategy_transient_recommends_retry() {
        let s = RecoveryStrategy::default();
        let action = s.recommend(ErrorCategory::Transient);
        assert!(matches!(action, RecoveryAction::Retry { .. }));
    }

    #[test]
    fn strategy_rate_limit_recommends_retry() {
        let s = RecoveryStrategy::default();
        let action = s.recommend(ErrorCategory::RateLimit);
        if let RecoveryAction::Retry { delay, .. } = action {
            assert_eq!(delay, Duration::from_secs(10));
        } else {
            panic!("Expected Retry");
        }
    }

    #[test]
    fn strategy_auth_recommends_ask_user() {
        let s = RecoveryStrategy::default();
        let action = s.recommend(ErrorCategory::Auth);
        assert!(matches!(action, RecoveryAction::AskUser { .. }));
    }

    #[test]
    fn strategy_permanent_recommends_abort() {
        let s = RecoveryStrategy::default();
        let action = s.recommend(ErrorCategory::Permanent);
        assert!(matches!(action, RecoveryAction::Abort { .. }));
    }

    #[test]
    fn strategy_unknown_recommends_fallback() {
        let s = RecoveryStrategy::default();
        let action = s.recommend(ErrorCategory::Unknown);
        assert!(matches!(action, RecoveryAction::Fallback { .. }));
    }

    #[test]
    fn strategy_timeout_recommends_retry() {
        let s = RecoveryStrategy::default();
        let action = s.recommend(ErrorCategory::Timeout);
        assert!(matches!(action, RecoveryAction::Retry { .. }));
    }

    #[test]
    fn strategy_with_attempts_escalates_transient() {
        let s = RecoveryStrategy::default();
        // Within limit
        let action = s.recommend_with_attempts(ErrorCategory::Transient, 1);
        assert!(matches!(action, RecoveryAction::Retry { .. }));
        // Exhausted
        let action = s.recommend_with_attempts(ErrorCategory::Transient, 3);
        assert!(matches!(action, RecoveryAction::Abort { .. }));
    }

    #[test]
    fn strategy_with_attempts_escalates_rate_limit() {
        let s = RecoveryStrategy::default();
        let action = s.recommend_with_attempts(ErrorCategory::RateLimit, 5);
        assert!(matches!(action, RecoveryAction::AskUser { .. }));
    }

    #[test]
    fn strategy_with_attempts_escalates_timeout() {
        let s = RecoveryStrategy::default();
        let action = s.recommend_with_attempts(ErrorCategory::Timeout, 2);
        assert!(matches!(action, RecoveryAction::Fallback { .. }));
    }

    #[test]
    fn strategy_with_attempts_no_escalation_for_permanent() {
        let s = RecoveryStrategy::default();
        // Permanent always aborts regardless of attempts
        let action = s.recommend_with_attempts(ErrorCategory::Permanent, 0);
        assert!(matches!(action, RecoveryAction::Abort { .. }));
    }

    // ── CircuitState ───────────────────────────────────────────────

    #[test]
    fn circuit_state_display() {
        assert_eq!(CircuitState::Closed.to_string(), "closed");
        assert_eq!(CircuitState::Open.to_string(), "open");
        assert_eq!(CircuitState::HalfOpen.to_string(), "half_open");
    }

    // ── CircuitBreaker ─────────────────────────────────────────────

    #[test]
    fn breaker_starts_closed() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.consecutive_failures(), 0);
        assert_eq!(cb.total_trips(), 0);
    }

    #[test]
    fn breaker_allows_when_closed() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert!(cb.allow_request());
    }

    #[test]
    fn breaker_trips_after_threshold() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert_eq!(cb.total_trips(), 1);
    }

    #[test]
    fn breaker_blocks_when_open() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            open_duration: Duration::from_secs(60),
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn breaker_success_resets_count() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.consecutive_failures(), 2);

        cb.record_success();
        assert_eq!(cb.consecutive_failures(), 0);
    }

    #[test]
    fn breaker_half_open_success_closes() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            open_duration: Duration::from_millis(0),
            success_threshold: 2,
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for open_duration (0ms) and allow request -> half_open
        assert!(cb.allow_request());
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen); // need 2 successes
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn breaker_half_open_failure_reopens() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            open_duration: Duration::from_millis(0),
            success_threshold: 2,
        });

        cb.record_failure();
        assert!(cb.allow_request()); // transitions to HalfOpen
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_failure(); // probe failed
        assert_eq!(cb.state(), CircuitState::Open);
        assert_eq!(cb.total_trips(), 2);
    }

    #[test]
    fn breaker_reset() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.consecutive_failures(), 0);
    }

    // ── DegradableFeature ──────────────────────────────────────────

    #[test]
    fn degradable_feature_display() {
        assert_eq!(DegradableFeature::SmartContext.to_string(), "smart_context");
        assert_eq!(
            DegradableFeature::ToolExecution.to_string(),
            "tool_execution"
        );
        assert_eq!(DegradableFeature::Streaming.to_string(), "streaming");
    }

    // ── FeaturePriority ────────────────────────────────────────────

    #[test]
    fn priority_ordering() {
        assert!(FeaturePriority::LOW < FeaturePriority::MEDIUM);
        assert!(FeaturePriority::MEDIUM < FeaturePriority::HIGH);
        assert!(FeaturePriority::HIGH < FeaturePriority::CORE);
    }

    // ── GracefulDegradation ────────────────────────────────────────

    #[test]
    fn degradation_starts_normal() {
        let gd = GracefulDegradation::new();
        assert_eq!(gd.level(), 0);
        assert!(gd.is_enabled(DegradableFeature::SmartContext));
        assert!(gd.is_enabled(DegradableFeature::ToolExecution));
        assert!(gd.disabled_features().is_empty());
    }

    #[test]
    fn degrade_level_1_disables_low_priority() {
        let mut gd = GracefulDegradation::new();
        let disabled = gd.degrade();
        assert_eq!(gd.level(), 1);
        // LOW priority features (SmartContext, CodeNavigation) should be disabled
        assert!(!gd.is_enabled(DegradableFeature::SmartContext));
        assert!(!gd.is_enabled(DegradableFeature::CodeNavigation));
        // MEDIUM and above still enabled
        assert!(gd.is_enabled(DegradableFeature::MemoryRetrieval));
        assert!(gd.is_enabled(DegradableFeature::ToolExecution));
        assert!(!disabled.is_empty());
    }

    #[test]
    fn degrade_level_2_disables_medium_priority() {
        let mut gd = GracefulDegradation::new();
        gd.degrade(); // level 1
        gd.degrade(); // level 2
        assert_eq!(gd.level(), 2);
        assert!(!gd.is_enabled(DegradableFeature::MemoryRetrieval));
        assert!(!gd.is_enabled(DegradableFeature::MultiAgent));
        // HIGH still enabled
        assert!(gd.is_enabled(DegradableFeature::Streaming));
        assert!(gd.is_enabled(DegradableFeature::ToolExecution));
    }

    #[test]
    fn degrade_level_3_disables_high_priority() {
        let mut gd = GracefulDegradation::new();
        gd.degrade();
        gd.degrade();
        gd.degrade();
        assert_eq!(gd.level(), 3);
        assert!(!gd.is_enabled(DegradableFeature::Streaming));
        // CORE still enabled
        assert!(gd.is_enabled(DegradableFeature::ToolExecution));
    }

    #[test]
    fn degrade_level_4_disables_everything() {
        let mut gd = GracefulDegradation::new();
        for _ in 0..4 {
            gd.degrade();
        }
        assert_eq!(gd.level(), 4);
        assert!(!gd.is_enabled(DegradableFeature::ToolExecution));
        assert!(gd.enabled_features().is_empty());
    }

    #[test]
    fn degrade_past_max_is_noop() {
        let mut gd = GracefulDegradation::new();
        for _ in 0..10 {
            gd.degrade();
        }
        assert_eq!(gd.level(), 4); // capped at max
    }

    #[test]
    fn recover_re_enables_features() {
        let mut gd = GracefulDegradation::new();
        gd.degrade(); // level 1
        assert!(!gd.is_enabled(DegradableFeature::SmartContext));

        let restored = gd.recover(); // back to level 0
        assert_eq!(gd.level(), 0);
        assert!(gd.is_enabled(DegradableFeature::SmartContext));
        assert!(!restored.is_empty());
    }

    #[test]
    fn recover_at_zero_is_noop() {
        let mut gd = GracefulDegradation::new();
        let restored = gd.recover();
        assert!(restored.is_empty());
        assert_eq!(gd.level(), 0);
    }

    #[test]
    fn reset_restores_all() {
        let mut gd = GracefulDegradation::new();
        gd.degrade();
        gd.degrade();
        gd.degrade();

        gd.reset();
        assert_eq!(gd.level(), 0);
        assert!(gd.disabled_features().is_empty());
        assert_eq!(gd.enabled_features().len(), 6);
    }

    #[test]
    fn manual_disable_enable() {
        let mut gd = GracefulDegradation::new();
        assert!(gd.is_enabled(DegradableFeature::Streaming));

        gd.disable_feature(DegradableFeature::Streaming);
        assert!(!gd.is_enabled(DegradableFeature::Streaming));

        gd.enable_feature(DegradableFeature::Streaming);
        assert!(gd.is_enabled(DegradableFeature::Streaming));
    }

    #[test]
    fn disabled_and_enabled_features_consistent() {
        let mut gd = GracefulDegradation::new();
        let total = gd.enabled_features().len() + gd.disabled_features().len();
        assert_eq!(total, 6);

        gd.degrade();
        let total = gd.enabled_features().len() + gd.disabled_features().len();
        assert_eq!(total, 6);
    }

    // ── Integration: classify + strategy ───────────────────────────

    #[test]
    fn end_to_end_classify_and_recover_transient() {
        let cat = ErrorClassifier::classify("503 Service Unavailable");
        let strategy = RecoveryStrategy::default();
        let action = strategy.recommend(cat);
        assert!(matches!(action, RecoveryAction::Retry { .. }));
    }

    #[test]
    fn end_to_end_classify_and_recover_auth() {
        let cat = ErrorClassifier::classify("Invalid API key");
        let strategy = RecoveryStrategy::default();
        let action = strategy.recommend(cat);
        assert!(matches!(action, RecoveryAction::AskUser { .. }));
    }

    #[test]
    fn end_to_end_classify_and_recover_permanent() {
        let cat = ErrorClassifier::classify("400 Bad Request: malformed JSON");
        let strategy = RecoveryStrategy::default();
        let action = strategy.recommend(cat);
        // "malformed" matches Permanent before Transient patterns are checked
        assert!(matches!(action, RecoveryAction::Abort { .. }));
    }

    #[test]
    fn end_to_end_status_and_recover() {
        let cat = ErrorClassifier::classify_status(429);
        let strategy = RecoveryStrategy::default();
        let action = strategy.recommend_with_attempts(cat, 0);
        assert!(matches!(action, RecoveryAction::Retry { .. }));
    }
}
