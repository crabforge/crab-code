//! Agent health monitoring via heartbeat mechanism.
//!
//! Each agent periodically reports a heartbeat. The `HealthMonitor` detects
//! agents that have missed too many beats and marks them as unhealthy.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Health status of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Agent is responding within the expected interval.
    Healthy,
    /// Agent has missed one or more heartbeats but is not yet timed out.
    Degraded,
    /// Agent has exceeded the timeout threshold — considered stuck or dead.
    Unresponsive,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unresponsive => write!(f, "unresponsive"),
        }
    }
}

/// Configuration for the health monitor.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Expected interval between heartbeats.
    pub heartbeat_interval: Duration,
    /// After this many missed intervals, mark as Degraded.
    pub degraded_threshold: u32,
    /// After this many missed intervals, mark as Unresponsive.
    pub unresponsive_threshold: u32,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(10),
            degraded_threshold: 2,
            unresponsive_threshold: 5,
        }
    }
}

/// Snapshot of an agent's health at a point in time.
#[derive(Debug, Clone)]
pub struct AgentHealth {
    pub agent_name: String,
    pub status: HealthStatus,
    pub last_heartbeat: Instant,
    pub missed_beats: u32,
}

/// Per-agent heartbeat state.
struct HeartbeatRecord {
    last_beat: Instant,
}

/// Monitors agent health by tracking heartbeats.
pub struct HealthMonitor {
    config: HealthConfig,
    records: HashMap<String, HeartbeatRecord>,
}

impl HealthMonitor {
    /// Create a new monitor with the given config.
    #[must_use]
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            records: HashMap::new(),
        }
    }

    /// Create a new monitor with default config.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(HealthConfig::default())
    }

    /// Register an agent (initializes heartbeat to now).
    pub fn register(&mut self, agent_name: impl Into<String>) {
        self.records.insert(
            agent_name.into(),
            HeartbeatRecord {
                last_beat: Instant::now(),
            },
        );
    }

    /// Unregister an agent.
    pub fn unregister(&mut self, agent_name: &str) -> bool {
        self.records.remove(agent_name).is_some()
    }

    /// Record a heartbeat from an agent.
    pub fn heartbeat(&mut self, agent_name: &str) {
        if let Some(record) = self.records.get_mut(agent_name) {
            record.last_beat = Instant::now();
        }
    }

    /// Record a heartbeat with a specific timestamp (for testing).
    #[cfg(test)]
    fn heartbeat_at(&mut self, agent_name: &str, at: Instant) {
        if let Some(record) = self.records.get_mut(agent_name) {
            record.last_beat = at;
        }
    }

    /// Check the health status of a specific agent.
    #[must_use]
    pub fn check(&self, agent_name: &str) -> Option<AgentHealth> {
        self.check_at(agent_name, Instant::now())
    }

    /// Check health at a specific time (for testing).
    fn check_at(&self, agent_name: &str, now: Instant) -> Option<AgentHealth> {
        let record = self.records.get(agent_name)?;
        let elapsed = now.duration_since(record.last_beat);
        let missed = if self.config.heartbeat_interval.as_nanos() > 0 {
            u32::try_from(elapsed.as_nanos() / self.config.heartbeat_interval.as_nanos())
                .unwrap_or(u32::MAX)
        } else {
            0
        };

        let status = if missed >= self.config.unresponsive_threshold {
            HealthStatus::Unresponsive
        } else if missed >= self.config.degraded_threshold {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        Some(AgentHealth {
            agent_name: agent_name.to_string(),
            status,
            last_heartbeat: record.last_beat,
            missed_beats: missed,
        })
    }

    /// Check all agents and return those that are unhealthy (Degraded or Unresponsive).
    #[must_use]
    pub fn unhealthy_agents(&self) -> Vec<AgentHealth> {
        self.unhealthy_agents_at(Instant::now())
    }

    fn unhealthy_agents_at(&self, now: Instant) -> Vec<AgentHealth> {
        self.records
            .keys()
            .filter_map(|name| {
                let health = self.check_at(name, now)?;
                if health.status == HealthStatus::Healthy {
                    None
                } else {
                    Some(health)
                }
            })
            .collect()
    }

    /// Get names of all unresponsive agents.
    #[must_use]
    pub fn unresponsive_agents(&self) -> Vec<String> {
        self.unresponsive_agents_at(Instant::now())
    }

    fn unresponsive_agents_at(&self, now: Instant) -> Vec<String> {
        self.records
            .keys()
            .filter(|name| {
                self.check_at(name, now)
                    .is_some_and(|h| h.status == HealthStatus::Unresponsive)
            })
            .cloned()
            .collect()
    }

    /// Number of registered agents.
    #[must_use]
    pub fn agent_count(&self) -> usize {
        self.records.len()
    }

    /// Get the config.
    #[must_use]
    pub fn config(&self) -> &HealthConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HealthConfig {
        HealthConfig {
            heartbeat_interval: Duration::from_secs(10),
            degraded_threshold: 2,
            unresponsive_threshold: 5,
        }
    }

    #[test]
    fn monitor_new_empty() {
        let mon = HealthMonitor::new(test_config());
        assert_eq!(mon.agent_count(), 0);
        assert!(mon.check("nobody").is_none());
    }

    #[test]
    fn monitor_with_defaults() {
        let mon = HealthMonitor::with_defaults();
        assert_eq!(mon.config().heartbeat_interval, Duration::from_secs(10));
        assert_eq!(mon.config().degraded_threshold, 2);
        assert_eq!(mon.config().unresponsive_threshold, 5);
    }

    #[test]
    fn register_and_check_healthy() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");

        let health = mon.check("alice").unwrap();
        assert_eq!(health.status, HealthStatus::Healthy);
        assert_eq!(health.missed_beats, 0);
        assert_eq!(health.agent_name, "alice");
    }

    #[test]
    fn unregister_agent() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");
        assert!(mon.unregister("alice"));
        assert!(mon.check("alice").is_none());
        assert!(!mon.unregister("alice"));
    }

    #[test]
    fn heartbeat_resets_timer() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");

        // Simulate time passing by setting last_beat in the past
        let past = Instant::now() - Duration::from_secs(30);
        mon.heartbeat_at("alice", past);

        let health = mon.check("alice").unwrap();
        assert_eq!(health.status, HealthStatus::Degraded);

        // Heartbeat resets
        mon.heartbeat("alice");
        let health = mon.check("alice").unwrap();
        assert_eq!(health.status, HealthStatus::Healthy);
    }

    #[test]
    fn degraded_after_threshold() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");

        // 2 missed intervals = 20s with 10s interval → Degraded
        let past = Instant::now() - Duration::from_secs(25);
        mon.heartbeat_at("alice", past);

        let health = mon.check("alice").unwrap();
        assert_eq!(health.status, HealthStatus::Degraded);
        assert!(health.missed_beats >= 2);
    }

    #[test]
    fn unresponsive_after_threshold() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");

        // 5 missed intervals = 50s → Unresponsive
        let past = Instant::now() - Duration::from_secs(55);
        mon.heartbeat_at("alice", past);

        let health = mon.check("alice").unwrap();
        assert_eq!(health.status, HealthStatus::Unresponsive);
        assert!(health.missed_beats >= 5);
    }

    #[test]
    fn unhealthy_agents_list() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");
        mon.register("bob");
        mon.register("charlie");

        let now = Instant::now();
        // alice: healthy (just registered)
        // bob: degraded (25s ago)
        mon.heartbeat_at("bob", now - Duration::from_secs(25));
        // charlie: unresponsive (60s ago)
        mon.heartbeat_at("charlie", now - Duration::from_secs(60));

        let unhealthy = mon.unhealthy_agents_at(now);
        assert_eq!(unhealthy.len(), 2);

        let names: Vec<_> = unhealthy.iter().map(|h| h.agent_name.as_str()).collect();
        assert!(names.contains(&"bob"));
        assert!(names.contains(&"charlie"));
    }

    #[test]
    fn unresponsive_agents_list() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");
        mon.register("bob");

        let now = Instant::now();
        mon.heartbeat_at("bob", now - Duration::from_secs(60));

        let unresponsive = mon.unresponsive_agents_at(now);
        assert_eq!(unresponsive.len(), 1);
        assert_eq!(unresponsive[0], "bob");
    }

    #[test]
    fn all_healthy_returns_empty_unhealthy() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");
        mon.register("bob");

        assert!(mon.unhealthy_agents().is_empty());
    }

    #[test]
    fn health_status_display() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(HealthStatus::Unresponsive.to_string(), "unresponsive");
    }

    #[test]
    fn health_config_default() {
        let config = HealthConfig::default();
        assert_eq!(config.heartbeat_interval, Duration::from_secs(10));
        assert_eq!(config.degraded_threshold, 2);
        assert_eq!(config.unresponsive_threshold, 5);
    }

    #[test]
    fn health_config_clone() {
        let config = HealthConfig {
            heartbeat_interval: Duration::from_secs(5),
            degraded_threshold: 3,
            unresponsive_threshold: 10,
        };
        let cloned = config.clone();
        assert_eq!(cloned.heartbeat_interval, Duration::from_secs(5));
    }

    #[test]
    fn heartbeat_nonexistent_agent_is_noop() {
        let mut mon = HealthMonitor::new(test_config());
        mon.heartbeat("nobody"); // should not panic
        assert!(mon.check("nobody").is_none());
    }

    #[test]
    fn multiple_agents_independent() {
        let mut mon = HealthMonitor::new(test_config());
        mon.register("alice");
        mon.register("bob");
        assert_eq!(mon.agent_count(), 2);

        let now = Instant::now();
        mon.heartbeat_at("bob", now - Duration::from_secs(55));

        // alice should be healthy
        let h_alice = mon.check_at("alice", now).unwrap();
        assert_eq!(h_alice.status, HealthStatus::Healthy);

        // bob should be unresponsive
        let h_bob = mon.check_at("bob", now).unwrap();
        assert_eq!(h_bob.status, HealthStatus::Unresponsive);
    }
}
