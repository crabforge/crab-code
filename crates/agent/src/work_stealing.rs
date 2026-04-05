//! Work-stealing scheduler for agent task queues.
//!
//! Each agent has a local task queue. When an agent becomes idle, it can
//! "steal" tasks from the back of another agent's queue. This avoids
//! the problem of one agent being overloaded while others sit idle.

use std::collections::{HashMap, VecDeque};

/// A pending task in the work-stealing queue.
#[derive(Debug, Clone)]
pub struct QueuedTask {
    pub task_id: String,
    pub prompt: String,
    /// Optional capability required for this task.
    pub required_capability: Option<String>,
}

/// Per-agent task queue used by the work-stealing scheduler.
#[derive(Debug, Default)]
struct AgentQueue {
    tasks: VecDeque<QueuedTask>,
}

/// Work-stealing scheduler.
///
/// Maintains a per-agent queue of pending tasks. Idle agents can steal
/// from the back of busy agents' queues.
pub struct WorkStealingScheduler {
    queues: HashMap<String, AgentQueue>,
}

impl WorkStealingScheduler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
        }
    }

    /// Register an agent with an empty queue.
    pub fn register(&mut self, agent_name: impl Into<String>) {
        self.queues.entry(agent_name.into()).or_default();
    }

    /// Unregister an agent. Returns any remaining tasks in their queue.
    pub fn unregister(&mut self, agent_name: &str) -> Vec<QueuedTask> {
        self.queues
            .remove(agent_name)
            .map(|q| q.tasks.into_iter().collect())
            .unwrap_or_default()
    }

    /// Push a task to the front of an agent's queue (FIFO — agent processes from front).
    pub fn enqueue(&mut self, agent_name: &str, task: QueuedTask) -> bool {
        if let Some(queue) = self.queues.get_mut(agent_name) {
            queue.tasks.push_back(task);
            true
        } else {
            false
        }
    }

    /// Pop a task from the front of an agent's own queue.
    pub fn dequeue(&mut self, agent_name: &str) -> Option<QueuedTask> {
        self.queues
            .get_mut(agent_name)
            .and_then(|q| q.tasks.pop_front())
    }

    /// Attempt to steal a task from the busiest other agent.
    ///
    /// Steals from the **back** of the victim's queue (the most recently
    /// added task, which the victim hasn't started yet).
    ///
    /// Returns `Some((victim_name, task))` if a steal was successful.
    pub fn steal(&mut self, thief: &str) -> Option<(String, QueuedTask)> {
        // Find the busiest agent that is not the thief
        let victim_name = self
            .queues
            .iter()
            .filter(|(name, q)| *name != thief && q.tasks.len() > 1)
            .max_by_key(|(_, q)| q.tasks.len())
            .map(|(name, _)| name.clone())?;

        let task = self
            .queues
            .get_mut(&victim_name)
            .and_then(|q| q.tasks.pop_back())?;

        Some((victim_name, task))
    }

    /// Try to steal a task, but only if the thief's queue is empty
    /// and the victim has at least `min_victim_depth` tasks.
    pub fn steal_if_idle(
        &mut self,
        thief: &str,
        min_victim_depth: usize,
    ) -> Option<(String, QueuedTask)> {
        // Only steal if the thief has no work
        if self.queues.get(thief).is_some_and(|q| !q.tasks.is_empty()) {
            return None;
        }

        let victim_name = self
            .queues
            .iter()
            .filter(|(name, q)| *name != thief && q.tasks.len() >= min_victim_depth)
            .max_by_key(|(_, q)| q.tasks.len())
            .map(|(name, _)| name.clone())?;

        let task = self
            .queues
            .get_mut(&victim_name)
            .and_then(|q| q.tasks.pop_back())?;

        Some((victim_name, task))
    }

    /// Get the queue depth for a specific agent.
    #[must_use]
    pub fn queue_depth(&self, agent_name: &str) -> usize {
        self.queues.get(agent_name).map_or(0, |q| q.tasks.len())
    }

    /// Get queue depths for all agents.
    #[must_use]
    pub fn all_depths(&self) -> HashMap<String, usize> {
        self.queues
            .iter()
            .map(|(name, q)| (name.clone(), q.tasks.len()))
            .collect()
    }

    /// Total number of queued tasks across all agents.
    #[must_use]
    pub fn total_queued(&self) -> usize {
        self.queues.values().map(|q| q.tasks.len()).sum()
    }

    /// Number of registered agents.
    #[must_use]
    pub fn agent_count(&self) -> usize {
        self.queues.len()
    }

    /// Redistribute all tasks evenly across agents (rebalance).
    pub fn rebalance(&mut self) {
        if self.queues.is_empty() {
            return;
        }

        // Collect all tasks
        let all_tasks: Vec<QueuedTask> = self
            .queues
            .values_mut()
            .flat_map(|q| q.tasks.drain(..))
            .collect();

        // Distribute round-robin
        let agent_names: Vec<String> = self.queues.keys().cloned().collect();
        for (idx, task) in all_tasks.into_iter().enumerate() {
            let name = &agent_names[idx % agent_names.len()];
            if let Some(queue) = self.queues.get_mut(name) {
                queue.tasks.push_back(task);
            }
        }
    }
}

impl Default for WorkStealingScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> QueuedTask {
        QueuedTask {
            task_id: id.into(),
            prompt: format!("do {id}"),
            required_capability: None,
        }
    }

    #[test]
    fn scheduler_new_empty() {
        let s = WorkStealingScheduler::new();
        assert_eq!(s.agent_count(), 0);
        assert_eq!(s.total_queued(), 0);
    }

    #[test]
    fn scheduler_default() {
        let s = WorkStealingScheduler::default();
        assert_eq!(s.agent_count(), 0);
    }

    #[test]
    fn register_and_enqueue() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        assert_eq!(s.agent_count(), 1);
        assert_eq!(s.queue_depth("alice"), 0);

        assert!(s.enqueue("alice", task("t1")));
        assert!(s.enqueue("alice", task("t2")));
        assert_eq!(s.queue_depth("alice"), 2);
        assert_eq!(s.total_queued(), 2);
    }

    #[test]
    fn enqueue_unregistered_fails() {
        let mut s = WorkStealingScheduler::new();
        assert!(!s.enqueue("nobody", task("t1")));
    }

    #[test]
    fn dequeue_fifo() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.enqueue("alice", task("t1"));
        s.enqueue("alice", task("t2"));
        s.enqueue("alice", task("t3"));

        assert_eq!(s.dequeue("alice").unwrap().task_id, "t1");
        assert_eq!(s.dequeue("alice").unwrap().task_id, "t2");
        assert_eq!(s.dequeue("alice").unwrap().task_id, "t3");
        assert!(s.dequeue("alice").is_none());
    }

    #[test]
    fn unregister_returns_remaining() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.enqueue("alice", task("t1"));
        s.enqueue("alice", task("t2"));

        let remaining = s.unregister("alice");
        assert_eq!(remaining.len(), 2);
        assert_eq!(s.agent_count(), 0);
    }

    #[test]
    fn unregister_empty() {
        let mut s = WorkStealingScheduler::new();
        let remaining = s.unregister("nobody");
        assert!(remaining.is_empty());
    }

    // ─── Work stealing ───

    #[test]
    fn steal_from_busiest() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");
        s.register("charlie");

        // alice: 5 tasks, bob: 2 tasks, charlie: 0
        for i in 0..5 {
            s.enqueue("alice", task(&format!("a{i}")));
        }
        for i in 0..2 {
            s.enqueue("bob", task(&format!("b{i}")));
        }

        // charlie steals from alice (busiest)
        let (victim, stolen) = s.steal("charlie").unwrap();
        assert_eq!(victim, "alice");
        // Steals from the back (most recently enqueued)
        assert_eq!(stolen.task_id, "a4");
        assert_eq!(s.queue_depth("alice"), 4);
    }

    #[test]
    fn steal_requires_min_two_tasks() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");

        // alice has only 1 task — too few to steal
        s.enqueue("alice", task("a1"));

        assert!(s.steal("bob").is_none());
    }

    #[test]
    fn steal_from_self_excluded() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");

        for i in 0..5 {
            s.enqueue("alice", task(&format!("a{i}")));
        }

        // alice can't steal from herself
        assert!(s.steal("alice").is_none());
    }

    #[test]
    fn steal_if_idle_only_when_empty() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");

        for i in 0..5 {
            s.enqueue("alice", task(&format!("a{i}")));
        }
        s.enqueue("bob", task("b1")); // bob has work

        // bob is not idle — should not steal
        assert!(s.steal_if_idle("bob", 2).is_none());

        // Make bob idle
        s.dequeue("bob");
        assert_eq!(s.queue_depth("bob"), 0);

        // Now bob can steal
        let (victim, _stolen) = s.steal_if_idle("bob", 2).unwrap();
        assert_eq!(victim, "alice");
    }

    #[test]
    fn steal_if_idle_respects_min_depth() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");

        // alice has 2 tasks
        s.enqueue("alice", task("a1"));
        s.enqueue("alice", task("a2"));

        // Require min_victim_depth = 3 → no steal
        assert!(s.steal_if_idle("bob", 3).is_none());

        // Require min_victim_depth = 2 → steal
        assert!(s.steal_if_idle("bob", 2).is_some());
    }

    // ─── Rebalance ───

    #[test]
    fn rebalance_distributes_evenly() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");
        s.register("charlie");

        // Load all tasks onto alice
        for i in 0..9 {
            s.enqueue("alice", task(&format!("t{i}")));
        }

        assert_eq!(s.queue_depth("alice"), 9);
        assert_eq!(s.queue_depth("bob"), 0);
        assert_eq!(s.queue_depth("charlie"), 0);

        s.rebalance();

        // Should be 3 each
        assert_eq!(s.queue_depth("alice"), 3);
        assert_eq!(s.queue_depth("bob"), 3);
        assert_eq!(s.queue_depth("charlie"), 3);
        assert_eq!(s.total_queued(), 9);
    }

    #[test]
    fn rebalance_uneven() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");

        for i in 0..5 {
            s.enqueue("alice", task(&format!("t{i}")));
        }

        s.rebalance();

        // 5 tasks, 2 agents: one gets 3, the other 2
        let depths = s.all_depths();
        let total: usize = depths.values().sum();
        assert_eq!(total, 5);
        // Each should have at least 2
        for d in depths.values() {
            assert!(*d >= 2);
            assert!(*d <= 3);
        }
    }

    #[test]
    fn rebalance_empty_is_noop() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.rebalance(); // no panic
        assert_eq!(s.total_queued(), 0);
    }

    #[test]
    fn rebalance_no_agents_is_noop() {
        let mut s = WorkStealingScheduler::new();
        s.rebalance(); // no panic
    }

    // ─── All depths ───

    #[test]
    fn all_depths() {
        let mut s = WorkStealingScheduler::new();
        s.register("alice");
        s.register("bob");
        s.enqueue("alice", task("t1"));
        s.enqueue("alice", task("t2"));
        s.enqueue("bob", task("t3"));

        let depths = s.all_depths();
        assert_eq!(depths.get("alice"), Some(&2));
        assert_eq!(depths.get("bob"), Some(&1));
    }

    #[test]
    fn queue_depth_unregistered() {
        let s = WorkStealingScheduler::new();
        assert_eq!(s.queue_depth("nobody"), 0);
    }

    // ─── Queued task ───

    #[test]
    fn queued_task_with_capability() {
        let t = QueuedTask {
            task_id: "t1".into(),
            prompt: "do stuff".into(),
            required_capability: Some("frontend".into()),
        };
        assert_eq!(t.required_capability.as_deref(), Some("frontend"));
    }

    #[test]
    fn queued_task_clone() {
        let t = task("t1");
        let t2 = t.clone();
        assert_eq!(t2.task_id, "t1");
    }
}
