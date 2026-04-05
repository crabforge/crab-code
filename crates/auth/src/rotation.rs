//! Credential rotation — policy-based rotation with grace periods and logging.

use std::time::{Duration, Instant};

// ── Policy ──────────────────────────────────────────────────────────────

/// Policy controlling when and how credentials are rotated.
#[derive(Debug, Clone)]
pub struct RotationPolicy {
    /// How often credentials should be rotated.
    pub interval: Duration,
    /// Grace period after expiry during which old credentials still work.
    pub grace_period: Duration,
    /// Whether rotation should happen automatically.
    pub auto_rotate: bool,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(24 * 3600), // 24 hours
            grace_period: Duration::from_secs(3600),  // 1 hour
            auto_rotate: true,
        }
    }
}

// ── State ───────────────────────────────────────────────────────────────

/// Current lifecycle state of a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationState {
    /// Credential is current and valid.
    Current,
    /// Credential is approaching expiry (within grace period).
    Expiring,
    /// Credential has expired.
    Expired,
    /// Rotation is in progress.
    Rotating,
}

impl std::fmt::Display for RotationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => f.write_str("current"),
            Self::Expiring => f.write_str("expiring"),
            Self::Expired => f.write_str("expired"),
            Self::Rotating => f.write_str("rotating"),
        }
    }
}

// ── Tracked credential ──────────────────────────────────────────────────

/// A credential with rotation tracking metadata.
#[derive(Debug, Clone)]
pub struct TrackedCredential {
    /// An opaque identifier for this credential (e.g. key ID, token prefix).
    pub id: String,
    /// When this credential was issued / last rotated.
    pub issued_at: Instant,
    /// The rotation policy applied to this credential.
    pub policy: RotationPolicy,
    /// Current state.
    pub state: RotationState,
}

impl TrackedCredential {
    /// Create a new tracked credential issued right now.
    #[must_use]
    pub fn new(id: String, policy: RotationPolicy) -> Self {
        Self {
            id,
            issued_at: Instant::now(),
            policy,
            state: RotationState::Current,
        }
    }

    /// Create with a specific issue time (for testing).
    #[must_use]
    pub fn with_issued_at(id: String, policy: RotationPolicy, issued_at: Instant) -> Self {
        Self {
            id,
            issued_at,
            policy,
            state: RotationState::Current,
        }
    }

    /// Age of this credential.
    #[must_use]
    pub fn age(&self) -> Duration {
        self.issued_at.elapsed()
    }

    /// Check whether this credential needs rotation based on its policy.
    #[must_use]
    pub fn needs_rotation(&self) -> bool {
        self.state == RotationState::Rotating || self.age() >= self.policy.interval
    }

    /// Compute the current rotation state from age and policy.
    #[must_use]
    pub fn computed_state(&self) -> RotationState {
        if self.state == RotationState::Rotating {
            return RotationState::Rotating;
        }
        let age = self.age();
        if age >= self.policy.interval + self.policy.grace_period {
            RotationState::Expired
        } else if age >= self.policy.interval {
            RotationState::Expiring
        } else {
            RotationState::Current
        }
    }

    /// Update the state to match the computed value.
    pub fn refresh_state(&mut self) {
        if self.state != RotationState::Rotating {
            self.state = self.computed_state();
        }
    }

    /// Mark the credential as being rotated.
    pub fn begin_rotation(&mut self) {
        self.state = RotationState::Rotating;
    }

    /// Complete rotation — resets `issued_at` and state to Current.
    pub fn complete_rotation(&mut self) {
        self.issued_at = Instant::now();
        self.state = RotationState::Current;
    }
}

// ── Rotation log ────────────────────────────────────────────────────────

/// A single rotation event.
#[derive(Debug, Clone)]
pub struct RotationEvent {
    /// Credential identifier.
    pub credential_id: String,
    /// When the rotation happened (system time for logging).
    pub timestamp_secs: u64,
    /// Whether the rotation succeeded.
    pub success: bool,
    /// Optional details or error message.
    pub details: String,
}

/// In-memory log of rotation events.
#[derive(Debug, Default)]
pub struct RotationLog {
    events: Vec<RotationEvent>,
}

impl RotationLog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a rotation event.
    pub fn record(&mut self, event: RotationEvent) {
        self.events.push(event);
    }

    /// Record a successful rotation.
    pub fn record_success(&mut self, credential_id: &str, details: &str) {
        self.record(RotationEvent {
            credential_id: credential_id.to_string(),
            timestamp_secs: now_secs(),
            success: true,
            details: details.to_string(),
        });
    }

    /// Record a failed rotation.
    pub fn record_failure(&mut self, credential_id: &str, details: &str) {
        self.record(RotationEvent {
            credential_id: credential_id.to_string(),
            timestamp_secs: now_secs(),
            success: false,
            details: details.to_string(),
        });
    }

    /// All events (oldest first).
    #[must_use]
    pub fn events(&self) -> &[RotationEvent] {
        &self.events
    }

    /// Events for a specific credential.
    #[must_use]
    pub fn events_for(&self, credential_id: &str) -> Vec<&RotationEvent> {
        self.events
            .iter()
            .filter(|e| e.credential_id == credential_id)
            .collect()
    }

    /// Count of successful rotations.
    #[must_use]
    pub fn success_count(&self) -> usize {
        self.events.iter().filter(|e| e.success).count()
    }

    /// Count of failed rotations.
    #[must_use]
    pub fn failure_count(&self) -> usize {
        self.events.iter().filter(|e| !e.success).count()
    }

    /// Clear all events.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn short_policy() -> RotationPolicy {
        RotationPolicy {
            interval: Duration::from_millis(50),
            grace_period: Duration::from_millis(50),
            auto_rotate: true,
        }
    }

    // ── RotationPolicy ──────────────────────────────────────────────────

    #[test]
    fn default_policy() {
        let p = RotationPolicy::default();
        assert_eq!(p.interval, Duration::from_secs(24 * 3600));
        assert_eq!(p.grace_period, Duration::from_secs(3600));
        assert!(p.auto_rotate);
    }

    // ── RotationState ───────────────────────────────────────────────────

    #[test]
    fn state_display() {
        assert_eq!(RotationState::Current.to_string(), "current");
        assert_eq!(RotationState::Expiring.to_string(), "expiring");
        assert_eq!(RotationState::Expired.to_string(), "expired");
        assert_eq!(RotationState::Rotating.to_string(), "rotating");
    }

    // ── TrackedCredential ───────────────────────────────────────────────

    #[test]
    fn new_credential_is_current() {
        let cred = TrackedCredential::new("key-1".into(), RotationPolicy::default());
        assert_eq!(cred.state, RotationState::Current);
        assert!(!cred.needs_rotation());
        assert_eq!(cred.computed_state(), RotationState::Current);
    }

    #[test]
    fn credential_needs_rotation_after_interval() {
        let policy = short_policy();
        let issued = Instant::now() - Duration::from_millis(100);
        let cred = TrackedCredential::with_issued_at("key-1".into(), policy, issued);
        assert!(cred.needs_rotation());
    }

    #[test]
    fn credential_expiring_state() {
        let policy = short_policy();
        // Age = 60ms, interval = 50ms, grace = 50ms → expiring (50..100)
        let issued = Instant::now() - Duration::from_millis(60);
        let cred = TrackedCredential::with_issued_at("key-1".into(), policy, issued);
        assert_eq!(cred.computed_state(), RotationState::Expiring);
    }

    #[test]
    fn credential_expired_state() {
        let policy = short_policy();
        // Age = 150ms, interval + grace = 100ms → expired
        let issued = Instant::now() - Duration::from_millis(150);
        let cred = TrackedCredential::with_issued_at("key-1".into(), policy, issued);
        assert_eq!(cred.computed_state(), RotationState::Expired);
    }

    #[test]
    fn begin_and_complete_rotation() {
        let mut cred = TrackedCredential::new("key-1".into(), RotationPolicy::default());
        cred.begin_rotation();
        assert_eq!(cred.state, RotationState::Rotating);
        assert!(cred.needs_rotation());
        assert_eq!(cred.computed_state(), RotationState::Rotating);

        cred.complete_rotation();
        assert_eq!(cred.state, RotationState::Current);
        assert!(!cred.needs_rotation());
    }

    #[test]
    fn refresh_state_updates() {
        let policy = short_policy();
        let issued = Instant::now() - Duration::from_millis(60);
        let mut cred = TrackedCredential::with_issued_at("key-1".into(), policy, issued);
        assert_eq!(cred.state, RotationState::Current); // not yet refreshed
        cred.refresh_state();
        assert_eq!(cred.state, RotationState::Expiring);
    }

    #[test]
    fn refresh_state_does_not_override_rotating() {
        let mut cred = TrackedCredential::new("key-1".into(), RotationPolicy::default());
        cred.begin_rotation();
        cred.refresh_state();
        assert_eq!(cred.state, RotationState::Rotating);
    }

    #[test]
    fn credential_age_is_small() {
        let cred = TrackedCredential::new("key-1".into(), RotationPolicy::default());
        assert!(cred.age() < Duration::from_secs(1));
    }

    // ── RotationLog ─────────────────────────────────────────────────────

    #[test]
    fn log_new_is_empty() {
        let log = RotationLog::new();
        assert!(log.events().is_empty());
        assert_eq!(log.success_count(), 0);
        assert_eq!(log.failure_count(), 0);
    }

    #[test]
    fn log_record_success() {
        let mut log = RotationLog::new();
        log.record_success("key-1", "rotated ok");
        assert_eq!(log.events().len(), 1);
        assert!(log.events()[0].success);
        assert_eq!(log.success_count(), 1);
        assert_eq!(log.failure_count(), 0);
    }

    #[test]
    fn log_record_failure() {
        let mut log = RotationLog::new();
        log.record_failure("key-1", "network error");
        assert_eq!(log.events().len(), 1);
        assert!(!log.events()[0].success);
        assert_eq!(log.failure_count(), 1);
    }

    #[test]
    fn log_events_for_filters() {
        let mut log = RotationLog::new();
        log.record_success("key-1", "ok");
        log.record_success("key-2", "ok");
        log.record_failure("key-1", "err");

        let key1_events = log.events_for("key-1");
        assert_eq!(key1_events.len(), 2);
        let key2_events = log.events_for("key-2");
        assert_eq!(key2_events.len(), 1);
    }

    #[test]
    fn log_clear() {
        let mut log = RotationLog::new();
        log.record_success("key-1", "ok");
        log.clear();
        assert!(log.events().is_empty());
    }

    #[test]
    fn log_timestamp_is_recent() {
        let mut log = RotationLog::new();
        log.record_success("key-1", "ok");
        let ts = log.events()[0].timestamp_secs;
        let now = now_secs();
        assert!(now - ts < 5);
    }

    #[test]
    fn rotation_event_details() {
        let event = RotationEvent {
            credential_id: "key-1".to_string(),
            timestamp_secs: 1_700_000_000,
            success: true,
            details: "auto-rotated".to_string(),
        };
        assert_eq!(event.credential_id, "key-1");
        assert_eq!(event.details, "auto-rotated");
    }
}
