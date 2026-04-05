//! Per-agent metrics collection.
//!
//! Tracks task counts, durations, and error rates for each agent.
//! Thread-safe via interior mutability (`Arc<Mutex<...>>`-free — uses atomics
//! where possible and `Mutex` only for duration tracking).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Per-agent metrics snapshot (read-only view).
#[derive(Debug, Clone)]
pub struct AgentMetrics {
    pub agent_name: String,
    /// Total tasks completed (success + failure).
    pub tasks_completed: u64,
    /// Tasks that completed successfully.
    pub tasks_succeeded: u64,
    /// Tasks that failed.
    pub tasks_failed: u64,
    /// Total wall-clock time spent on tasks.
    pub total_duration: Duration,
    /// Error rate (0.0 – 1.0).
    pub error_rate: f64,
    /// Average task duration.
    pub avg_duration: Duration,
}

/// Mutable per-agent tracking state.
#[derive(Debug, Default)]
struct AgentRecord {
    tasks_succeeded: u64,
    tasks_failed: u64,
    total_duration: Duration,
    /// Currently running task start times, keyed by `task_id`.
    in_progress: HashMap<String, Instant>,
}

impl AgentRecord {
    fn snapshot(&self, agent_name: &str) -> AgentMetrics {
        let total = self.tasks_succeeded + self.tasks_failed;
        #[allow(clippy::cast_precision_loss)]
        let error_rate = if total > 0 {
            self.tasks_failed as f64 / total as f64
        } else {
            0.0
        };
        let avg_duration = if total > 0 {
            self.total_duration / u32::try_from(total).unwrap_or(u32::MAX)
        } else {
            Duration::ZERO
        };
        AgentMetrics {
            agent_name: agent_name.to_string(),
            tasks_completed: total,
            tasks_succeeded: self.tasks_succeeded,
            tasks_failed: self.tasks_failed,
            total_duration: self.total_duration,
            error_rate,
            avg_duration,
        }
    }
}

/// Collects metrics for all agents. Clone-friendly via `Arc`.
#[derive(Clone, Default)]
pub struct MetricsCollector {
    inner: Arc<Mutex<HashMap<String, AgentRecord>>>,
}

impl MetricsCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that an agent started working on a task.
    pub fn task_started(&self, agent_name: &str, task_id: &str) {
        let mut map = self.inner.lock().unwrap();
        map.entry(agent_name.to_string())
            .or_default()
            .in_progress
            .insert(task_id.to_string(), Instant::now());
    }

    /// Record that an agent completed a task successfully.
    pub fn task_succeeded(&self, agent_name: &str, task_id: &str) {
        let mut map = self.inner.lock().unwrap();
        let record = map.entry(agent_name.to_string()).or_default();
        let duration = record
            .in_progress
            .remove(task_id)
            .map_or(Duration::ZERO, |start| start.elapsed());
        record.tasks_succeeded += 1;
        record.total_duration += duration;
        drop(map);
    }

    /// Record that an agent's task failed.
    pub fn task_failed(&self, agent_name: &str, task_id: &str) {
        let mut map = self.inner.lock().unwrap();
        let record = map.entry(agent_name.to_string()).or_default();
        let duration = record
            .in_progress
            .remove(task_id)
            .map_or(Duration::ZERO, |start| start.elapsed());
        record.tasks_failed += 1;
        record.total_duration += duration;
        drop(map);
    }

    /// Record a completed task with an explicit duration (for external timing).
    pub fn record_completion(&self, agent_name: &str, success: bool, duration: Duration) {
        let mut map = self.inner.lock().unwrap();
        let record = map.entry(agent_name.to_string()).or_default();
        if success {
            record.tasks_succeeded += 1;
        } else {
            record.tasks_failed += 1;
        }
        record.total_duration += duration;
        drop(map);
    }

    /// Get a snapshot of a specific agent's metrics.
    #[must_use]
    pub fn get(&self, agent_name: &str) -> Option<AgentMetrics> {
        let map = self.inner.lock().unwrap();
        map.get(agent_name).map(|r| r.snapshot(agent_name))
    }

    /// Get snapshots for all tracked agents.
    #[must_use]
    pub fn all(&self) -> Vec<AgentMetrics> {
        let map = self.inner.lock().unwrap();
        map.iter().map(|(name, r)| r.snapshot(name)).collect()
    }

    /// Number of in-progress tasks for a given agent.
    #[must_use]
    pub fn in_progress_count(&self, agent_name: &str) -> usize {
        let map = self.inner.lock().unwrap();
        map.get(agent_name).map_or(0, |r| r.in_progress.len())
    }

    /// Total active task count across all agents.
    #[must_use]
    pub fn total_active(&self) -> usize {
        let map = self.inner.lock().unwrap();
        map.values().map(|r| r.in_progress.len()).sum()
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        let mut map = self.inner.lock().unwrap();
        map.clear();
    }
}

impl std::fmt::Debug for MetricsCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsCollector")
            .field("agents", &self.all().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_new_is_empty() {
        let mc = MetricsCollector::new();
        assert!(mc.all().is_empty());
        assert!(mc.get("alice").is_none());
        assert_eq!(mc.total_active(), 0);
    }

    #[test]
    fn task_lifecycle_success() {
        let mc = MetricsCollector::new();
        mc.task_started("alice", "t1");
        assert_eq!(mc.in_progress_count("alice"), 1);
        assert_eq!(mc.total_active(), 1);

        mc.task_succeeded("alice", "t1");
        assert_eq!(mc.in_progress_count("alice"), 0);

        let m = mc.get("alice").unwrap();
        assert_eq!(m.tasks_completed, 1);
        assert_eq!(m.tasks_succeeded, 1);
        assert_eq!(m.tasks_failed, 0);
        assert!((m.error_rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn task_lifecycle_failure() {
        let mc = MetricsCollector::new();
        mc.task_started("bob", "t1");
        mc.task_failed("bob", "t1");

        let m = mc.get("bob").unwrap();
        assert_eq!(m.tasks_completed, 1);
        assert_eq!(m.tasks_succeeded, 0);
        assert_eq!(m.tasks_failed, 1);
        assert!((m.error_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn mixed_success_and_failure() {
        let mc = MetricsCollector::new();
        for i in 0..8 {
            let tid = format!("t{i}");
            mc.task_started("alice", &tid);
            mc.task_succeeded("alice", &tid);
        }
        for i in 8..10 {
            let tid = format!("t{i}");
            mc.task_started("alice", &tid);
            mc.task_failed("alice", &tid);
        }

        let m = mc.get("alice").unwrap();
        assert_eq!(m.tasks_completed, 10);
        assert_eq!(m.tasks_succeeded, 8);
        assert_eq!(m.tasks_failed, 2);
        assert!((m.error_rate - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn record_completion_external_timing() {
        let mc = MetricsCollector::new();
        mc.record_completion("alice", true, Duration::from_secs(5));
        mc.record_completion("alice", true, Duration::from_secs(3));
        mc.record_completion("alice", false, Duration::from_secs(2));

        let m = mc.get("alice").unwrap();
        assert_eq!(m.tasks_completed, 3);
        assert_eq!(m.tasks_succeeded, 2);
        assert_eq!(m.tasks_failed, 1);
        assert_eq!(m.total_duration, Duration::from_secs(10));
        // avg = 10s / 3 = 3.333s
        assert!(m.avg_duration.as_secs() >= 3);
    }

    #[test]
    fn multiple_agents() {
        let mc = MetricsCollector::new();
        mc.record_completion("alice", true, Duration::from_secs(1));
        mc.record_completion("bob", true, Duration::from_secs(2));
        mc.record_completion("charlie", false, Duration::from_secs(1));

        let all = mc.all();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn multiple_in_progress() {
        let mc = MetricsCollector::new();
        mc.task_started("alice", "t1");
        mc.task_started("alice", "t2");
        mc.task_started("bob", "t3");
        assert_eq!(mc.in_progress_count("alice"), 2);
        assert_eq!(mc.in_progress_count("bob"), 1);
        assert_eq!(mc.total_active(), 3);

        mc.task_succeeded("alice", "t1");
        assert_eq!(mc.in_progress_count("alice"), 1);
        assert_eq!(mc.total_active(), 2);
    }

    #[test]
    fn reset_clears_all() {
        let mc = MetricsCollector::new();
        mc.record_completion("alice", true, Duration::from_secs(1));
        mc.record_completion("bob", true, Duration::from_secs(2));
        assert_eq!(mc.all().len(), 2);

        mc.reset();
        assert!(mc.all().is_empty());
        assert!(mc.get("alice").is_none());
    }

    #[test]
    fn clone_shares_state() {
        let mc1 = MetricsCollector::new();
        let mc2 = mc1.clone();

        mc1.record_completion("alice", true, Duration::from_secs(1));
        assert_eq!(mc2.get("alice").unwrap().tasks_completed, 1);
    }

    #[test]
    fn succeed_without_start_records_zero_duration() {
        let mc = MetricsCollector::new();
        mc.task_succeeded("alice", "phantom");

        let m = mc.get("alice").unwrap();
        assert_eq!(m.tasks_succeeded, 1);
        assert_eq!(m.total_duration, Duration::ZERO);
    }

    #[test]
    fn avg_duration_zero_when_no_tasks() {
        let mc = MetricsCollector::new();
        // Force an empty record
        mc.task_started("alice", "t1");
        // Remove the in-progress without completing — get a snapshot
        let m = mc.get("alice").unwrap();
        assert_eq!(m.tasks_completed, 0);
        assert_eq!(m.avg_duration, Duration::ZERO);
    }

    #[test]
    fn debug_format() {
        let mc = MetricsCollector::new();
        let s = format!("{mc:?}");
        assert!(s.contains("MetricsCollector"));
    }
}
