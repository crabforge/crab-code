//! Security audit log — structured auth event recording and querying.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Event types ────────────────────────────────────────────────────────

/// Category of authentication event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthEventType {
    /// User login / session start.
    Login,
    /// User logout / session end.
    Logout,
    /// OAuth / API token refresh.
    TokenRefresh,
    /// Credential rotation.
    KeyRotation,
    /// Permission or authorization check.
    PermissionCheck,
    /// Credential read / access.
    CredentialAccess,
}

impl std::fmt::Display for AuthEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Login => f.write_str("login"),
            Self::Logout => f.write_str("logout"),
            Self::TokenRefresh => f.write_str("token_refresh"),
            Self::KeyRotation => f.write_str("key_rotation"),
            Self::PermissionCheck => f.write_str("permission_check"),
            Self::CredentialAccess => f.write_str("credential_access"),
        }
    }
}

// ── Audit event ────────────────────────────────────────────────────────

/// A single security audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthAuditEvent {
    /// Monotonically increasing event ID within this log instance.
    pub id: u64,
    /// Unix timestamp (seconds).
    pub timestamp_secs: u64,
    /// Type of event.
    pub event_type: AuthEventType,
    /// Whether the action succeeded.
    pub success: bool,
    /// Identifier of the credential / principal involved.
    pub principal: String,
    /// Free-form detail or error message.
    pub detail: String,
    /// Optional key-value metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

// ── Query filter ───────────────────────────────────────────────────────

/// Filter predicate for querying audit events.
#[derive(Debug, Default)]
pub struct AuditFilter {
    /// Only events of this type.
    pub event_type: Option<AuthEventType>,
    /// Only events for this principal.
    pub principal: Option<String>,
    /// Only successes (`true`) or failures (`false`).
    pub success: Option<bool>,
    /// Only events after this timestamp (inclusive).
    pub after: Option<u64>,
    /// Only events before this timestamp (inclusive).
    pub before: Option<u64>,
}

impl AuditFilter {
    fn matches(&self, event: &AuthAuditEvent) -> bool {
        if let Some(et) = &self.event_type
            && event.event_type != *et
        {
            return false;
        }
        if let Some(p) = &self.principal
            && event.principal != *p
        {
            return false;
        }
        if let Some(s) = self.success
            && event.success != s
        {
            return false;
        }
        if let Some(after) = self.after
            && event.timestamp_secs < after
        {
            return false;
        }
        if let Some(before) = self.before
            && event.timestamp_secs > before
        {
            return false;
        }
        true
    }
}

// ── Summary ────────────────────────────────────────────────────────────

/// Aggregate summary of audit events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditSummary {
    pub total_events: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub events_by_type: BTreeMap<String, usize>,
    pub unique_principals: usize,
}

// ── Audit log ──────────────────────────────────────────────────────────

/// In-memory security audit log.
#[derive(Debug, Default)]
pub struct AuthAuditLog {
    events: Vec<AuthAuditEvent>,
    next_id: u64,
}

impl AuthAuditLog {
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            next_id: 1,
        }
    }

    /// Record a new audit event. Returns the assigned event ID.
    pub fn record(
        &mut self,
        event_type: AuthEventType,
        success: bool,
        principal: &str,
        detail: &str,
    ) -> u64 {
        self.record_with_metadata(event_type, success, principal, detail, BTreeMap::new())
    }

    /// Record an event with additional metadata.
    pub fn record_with_metadata(
        &mut self,
        event_type: AuthEventType,
        success: bool,
        principal: &str,
        detail: &str,
        metadata: BTreeMap<String, String>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.events.push(AuthAuditEvent {
            id,
            timestamp_secs: now_secs(),
            event_type,
            success,
            principal: principal.to_string(),
            detail: detail.to_string(),
            metadata,
        });
        id
    }

    /// All recorded events (oldest first).
    #[must_use]
    pub fn events(&self) -> &[AuthAuditEvent] {
        &self.events
    }

    /// Total event count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the log is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Query events matching a filter.
    #[must_use]
    pub fn query(&self, filter: &AuditFilter) -> Vec<&AuthAuditEvent> {
        self.events.iter().filter(|e| filter.matches(e)).collect()
    }

    /// Get a single event by ID.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&AuthAuditEvent> {
        self.events.iter().find(|e| e.id == id)
    }

    /// Compute an aggregate summary.
    #[must_use]
    pub fn summary(&self) -> AuditSummary {
        let mut events_by_type: BTreeMap<String, usize> = BTreeMap::new();
        let mut principals = std::collections::HashSet::new();
        let mut success_count = 0usize;
        let mut failure_count = 0usize;

        for event in &self.events {
            *events_by_type
                .entry(event.event_type.to_string())
                .or_insert(0) += 1;
            principals.insert(&event.principal);
            if event.success {
                success_count += 1;
            } else {
                failure_count += 1;
            }
        }

        AuditSummary {
            total_events: self.events.len(),
            success_count,
            failure_count,
            events_by_type,
            unique_principals: principals.len(),
        }
    }

    /// Clear all events.
    pub fn clear(&mut self) {
        self.events.clear();
        self.next_id = 1;
    }

    /// Export events as JSON string.
    ///
    /// Returns `None` if serialization fails.
    #[must_use]
    pub fn export_json(&self) -> Option<String> {
        serde_json::to_string_pretty(&self.events).ok()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── AuthEventType ──────────────────────────────────────────────────

    #[test]
    fn event_type_display() {
        assert_eq!(AuthEventType::Login.to_string(), "login");
        assert_eq!(AuthEventType::Logout.to_string(), "logout");
        assert_eq!(AuthEventType::TokenRefresh.to_string(), "token_refresh");
        assert_eq!(AuthEventType::KeyRotation.to_string(), "key_rotation");
        assert_eq!(
            AuthEventType::PermissionCheck.to_string(),
            "permission_check"
        );
        assert_eq!(
            AuthEventType::CredentialAccess.to_string(),
            "credential_access"
        );
    }

    #[test]
    fn event_type_serde_roundtrip() {
        let et = AuthEventType::TokenRefresh;
        let json = serde_json::to_string(&et).unwrap();
        assert_eq!(json, r#""token_refresh""#);
        let back: AuthEventType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, et);
    }

    // ── AuthAuditLog ───────────────────────────────────────────────────

    #[test]
    fn new_log_is_empty() {
        let log = AuthAuditLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn record_and_retrieve() {
        let mut log = AuthAuditLog::new();
        let id = log.record(AuthEventType::Login, true, "user-1", "logged in");
        assert_eq!(id, 1);
        assert_eq!(log.len(), 1);

        let event = log.get(id).unwrap();
        assert_eq!(event.event_type, AuthEventType::Login);
        assert!(event.success);
        assert_eq!(event.principal, "user-1");
        assert_eq!(event.detail, "logged in");
    }

    #[test]
    fn record_assigns_incrementing_ids() {
        let mut log = AuthAuditLog::new();
        let id1 = log.record(AuthEventType::Login, true, "a", "");
        let id2 = log.record(AuthEventType::Logout, true, "a", "");
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn record_with_metadata() {
        let mut log = AuthAuditLog::new();
        let mut meta = BTreeMap::new();
        meta.insert("ip".to_string(), "127.0.0.1".to_string());
        let id = log.record_with_metadata(
            AuthEventType::CredentialAccess,
            true,
            "svc-account",
            "read key",
            meta,
        );
        let event = log.get(id).unwrap();
        assert_eq!(event.metadata.get("ip").unwrap(), "127.0.0.1");
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let log = AuthAuditLog::new();
        assert!(log.get(99).is_none());
    }

    // ── Query ──────────────────────────────────────────────────────────

    #[test]
    fn query_by_event_type() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "a", "");
        log.record(AuthEventType::Logout, true, "a", "");
        log.record(AuthEventType::Login, false, "b", "");

        let filter = AuditFilter {
            event_type: Some(AuthEventType::Login),
            ..Default::default()
        };
        let results = log.query(&filter);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn query_by_principal() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "alice", "");
        log.record(AuthEventType::Login, true, "bob", "");

        let filter = AuditFilter {
            principal: Some("alice".into()),
            ..Default::default()
        };
        assert_eq!(log.query(&filter).len(), 1);
    }

    #[test]
    fn query_by_success() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "a", "");
        log.record(AuthEventType::Login, false, "a", "");

        let filter = AuditFilter {
            success: Some(false),
            ..Default::default()
        };
        assert_eq!(log.query(&filter).len(), 1);
        assert!(!log.query(&filter)[0].success);
    }

    #[test]
    fn query_empty_filter_returns_all() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "a", "");
        log.record(AuthEventType::Logout, true, "b", "");
        assert_eq!(log.query(&AuditFilter::default()).len(), 2);
    }

    // ── Summary ────────────────────────────────────────────────────────

    #[test]
    fn summary_empty_log() {
        let log = AuthAuditLog::new();
        let s = log.summary();
        assert_eq!(s.total_events, 0);
        assert_eq!(s.success_count, 0);
        assert_eq!(s.failure_count, 0);
        assert_eq!(s.unique_principals, 0);
    }

    #[test]
    fn summary_counts() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "alice", "");
        log.record(AuthEventType::Login, false, "bob", "");
        log.record(AuthEventType::TokenRefresh, true, "alice", "");

        let s = log.summary();
        assert_eq!(s.total_events, 3);
        assert_eq!(s.success_count, 2);
        assert_eq!(s.failure_count, 1);
        assert_eq!(s.unique_principals, 2);
        assert_eq!(s.events_by_type.get("login"), Some(&2));
        assert_eq!(s.events_by_type.get("token_refresh"), Some(&1));
    }

    // ── Clear / export ─────────────────────────────────────────────────

    #[test]
    fn clear_resets_log() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "a", "");
        log.clear();
        assert!(log.is_empty());
        // IDs restart after clear
        let id = log.record(AuthEventType::Login, true, "a", "");
        assert_eq!(id, 1);
    }

    #[test]
    fn export_json_produces_valid_json() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "user", "ok");
        let json = log.export_json().unwrap();
        let parsed: Vec<AuthAuditEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].principal, "user");
    }

    #[test]
    fn event_serde_roundtrip() {
        let event = AuthAuditEvent {
            id: 1,
            timestamp_secs: 1_700_000_000,
            event_type: AuthEventType::KeyRotation,
            success: true,
            principal: "svc".into(),
            detail: "rotated".into(),
            metadata: BTreeMap::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuthAuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 1);
        assert_eq!(back.event_type, AuthEventType::KeyRotation);
    }

    #[test]
    fn timestamp_is_recent() {
        let mut log = AuthAuditLog::new();
        let id = log.record(AuthEventType::Login, true, "a", "");
        let event = log.get(id).unwrap();
        let now = now_secs();
        assert!(now - event.timestamp_secs < 5);
    }

    #[test]
    fn events_returns_ordered_slice() {
        let mut log = AuthAuditLog::new();
        log.record(AuthEventType::Login, true, "first", "");
        log.record(AuthEventType::Logout, true, "second", "");
        let events = log.events();
        assert_eq!(events[0].principal, "first");
        assert_eq!(events[1].principal, "second");
    }
}
