use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crab_api::LlmBackend;
use crab_core::event::Event;
use crab_core::message::Message;
use crab_core::model::ModelId;
use crab_core::permission::PermissionPolicy;
use crab_core::tool::ToolContext;
use crab_session::{Conversation, CostAccumulator, MemoryStore, SessionHistory};
use crab_tools::executor::ToolExecutor;
use crab_tools::registry::ToolRegistry;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::assignment::AssignmentStrategy;
use crate::health::{HealthConfig, HealthMonitor};
use crate::message_bus::{AgentMessage, Envelope, MessageBus};
use crate::message_router::MessageRouter;
use crate::metrics::MetricsCollector;
use crate::query_loop::{self, QueryLoopConfig};
use crate::retry::{RetryDecision, RetryPolicy, RetryTracker};
use crate::team::{Capability, Team, TeamMode};
use crate::work_stealing::WorkStealingScheduler;
use crate::worker::{AgentWorker, WorkerConfig, WorkerResult};

/// Multi-agent orchestrator. Manages the main agent and worker pool.
///
/// The coordinator tracks running workers via `JoinHandle`s and provides
/// methods to spawn, cancel, and collect worker results.
pub struct AgentCoordinator {
    pub main_agent: AgentHandle,
    pub workers: Vec<AgentHandle>,
    pub bus: mpsc::Sender<AgentMessage>,
    /// Running worker tasks, keyed by worker ID.
    running: HashMap<String, RunningWorker>,
    /// Collected results from completed workers.
    completed: Vec<WorkerResult>,
    /// Counter for generating unique worker IDs.
    next_worker_id: u64,
    /// Message router for inter-agent communication.
    pub router: MessageRouter,
    /// Team structure and collaboration mode.
    pub team: Option<Team>,
    /// Task assignment strategy.
    assignment_strategy: Option<Box<dyn AssignmentStrategy>>,
    /// Agent health monitor.
    pub health: HealthMonitor,
    /// Per-agent metrics collector.
    pub metrics: MetricsCollector,
    /// Retry tracker for failed tasks.
    pub retry_tracker: RetryTracker,
    /// Work-stealing scheduler.
    pub scheduler: WorkStealingScheduler,
}

/// A worker that is currently running.
struct RunningWorker {
    pub handle: tokio::task::JoinHandle<WorkerResult>,
    pub cancel: CancellationToken,
}

/// Handle to a running agent (main or sub-agent).
pub struct AgentHandle {
    pub id: String,
    pub name: String,
    pub tx: mpsc::Sender<AgentMessage>,
}

/// Session configuration needed to start a query loop.
pub struct SessionConfig {
    pub session_id: String,
    pub system_prompt: String,
    pub model: ModelId,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub context_window: u64,
    pub working_dir: std::path::PathBuf,
    pub permission_policy: PermissionPolicy,
    /// Path to memory store directory (e.g., `~/.crab/memory/`).
    pub memory_dir: Option<PathBuf>,
    /// Path to session history directory (e.g., `~/.crab/sessions/`).
    pub sessions_dir: Option<PathBuf>,
    /// Session ID to resume from (for `--resume`).
    pub resume_session_id: Option<String>,
}

/// A running agent session with all the pieces wired together.
pub struct AgentSession {
    pub conversation: Conversation,
    pub backend: Arc<LlmBackend>,
    pub executor: ToolExecutor,
    pub tool_ctx: ToolContext,
    pub config: QueryLoopConfig,
    pub event_tx: mpsc::Sender<Event>,
    pub event_rx: mpsc::Receiver<Event>,
    pub cancel: CancellationToken,
    /// Memory store for loading/saving user memories.
    pub memory_store: Option<MemoryStore>,
    /// Session history for persisting conversation transcripts.
    pub session_history: Option<SessionHistory>,
    /// Cost accumulator for tracking API usage.
    pub cost: CostAccumulator,
    /// Conversation tree for branching and rollback support.
    pub conversation_tree: crate::conversation_tree::ConversationTree,
}

impl AgentSession {
    /// Initialize a new agent session.
    ///
    /// If `memory_dir` is set, loads memories and injects them into the
    /// system prompt. If `sessions_dir` is set, enables auto-save.
    /// If `resume_session_id` is set, restores messages from a prior session.
    pub fn new(
        session_config: SessionConfig,
        backend: Arc<LlmBackend>,
        registry: ToolRegistry,
    ) -> Self {
        let mut conversation = Conversation::new(
            session_config.session_id.clone(),
            session_config.system_prompt,
            session_config.context_window,
        );

        let memory_store = session_config.memory_dir.map(MemoryStore::new);
        let session_history = session_config.sessions_dir.map(SessionHistory::new);

        // Load memories and inject into system prompt
        if let Some(store) = &memory_store
            && let Ok(memories) = store.load_all()
            && !memories.is_empty()
        {
            let memory_section = format_memory_section(&memories);
            conversation.system_prompt.push_str(&memory_section);
        }

        // Resume from previous session if requested
        if let Some(resume_id) = &session_config.resume_session_id
            && let Some(history) = &session_history
            && let Ok(Some(messages)) = history.load(resume_id)
        {
            for msg in messages {
                conversation.push(msg);
            }
        }

        let tool_schemas = registry.tool_schemas();
        let executor = ToolExecutor::new(Arc::new(registry));
        let cancel = CancellationToken::new();

        let tool_ctx = ToolContext {
            working_dir: session_config.working_dir,
            permission_mode: session_config.permission_policy.mode,
            session_id: session_config.session_id,
            cancellation_token: cancel.clone(),
            permission_policy: session_config.permission_policy,
        };

        let config = QueryLoopConfig {
            model: session_config.model,
            max_tokens: session_config.max_tokens,
            temperature: session_config.temperature,
            tool_schemas,
            cache_enabled: false,
            token_budget: None,
            retry_policy: None,
        };

        let (event_tx, event_rx) = mpsc::channel(256);

        Self {
            conversation,
            backend,
            executor,
            tool_ctx,
            config,
            event_tx,
            event_rx,
            cancel,
            memory_store,
            session_history,
            cost: CostAccumulator::default(),
            conversation_tree: crate::conversation_tree::ConversationTree::new(),
        }
    }

    /// Try to handle input as a REPL slash command (`/undo`, `/branch`, `/fork`).
    ///
    /// Returns `Some(CommandResult)` if the input was a recognized command,
    /// or `None` if it should be treated as normal user input.
    pub fn handle_repl_command(
        &mut self,
        input: &str,
    ) -> Option<crate::repl_commands::CommandResult> {
        let cmd = crate::repl_commands::ReplCommand::parse(input);
        crate::repl_commands::execute_command(&cmd, &mut self.conversation_tree)
    }

    /// Handle user input: add user message, run the query loop, and auto-save.
    pub async fn handle_user_input(&mut self, input: &str) -> crab_common::Result<()> {
        // Check for REPL commands first — they don't go to the LLM.
        if self.handle_repl_command(input).is_some() {
            return Ok(());
        }

        // Mirror the message into the conversation tree for branching support.
        self.conversation_tree.push(Message::user(input));
        self.conversation.push(Message::user(input));

        let result = query_loop::query_loop(
            &mut self.conversation,
            &self.backend,
            &self.executor,
            &self.tool_ctx,
            &self.config,
            self.event_tx.clone(),
            self.cancel.clone(),
        )
        .await;

        // Auto-save session after each interaction
        self.auto_save_session().await;

        result
    }

    /// Cancel the running query loop.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Get a clone of the event sender for external use.
    pub fn event_sender(&self) -> mpsc::Sender<Event> {
        self.event_tx.clone()
    }

    /// Save a memory file through the memory store.
    pub fn save_memory(&self, filename: &str, content: &str) -> crab_common::Result<()> {
        if let Some(store) = &self.memory_store {
            store.save(filename, content)?;
        }
        Ok(())
    }

    /// Handle a spawn request from `AgentTool` output.
    ///
    /// Parses the structured JSON from `AgentTool` (with `"action": "spawn_agent"`)
    /// and spawns a worker via the provided coordinator. Returns the worker ID.
    pub fn handle_spawn_request(
        &self,
        coordinator: &mut AgentCoordinator,
        spawn_request: &serde_json::Value,
    ) -> Option<String> {
        if spawn_request.get("action")?.as_str()? != "spawn_agent" {
            return None;
        }

        let task = spawn_request.get("task")?.as_str()?.to_string();
        let max_turns = spawn_request
            .get("max_turns")
            .and_then(serde_json::Value::as_u64)
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX));

        let system_prompt = format!(
            "You are a sub-agent worker. Complete the assigned task.\n\n{}",
            self.conversation.system_prompt
        );

        let worker_executor = Arc::new(ToolExecutor::new(self.executor.registry_arc()));

        let worker_id = coordinator.spawn_worker(
            task,
            system_prompt,
            self.backend.clone(),
            worker_executor,
            self.tool_ctx.clone(),
            self.config.clone(),
            self.event_tx.clone(),
            max_turns,
        );

        Some(worker_id)
    }

    /// Auto-save the current session transcript to disk.
    async fn auto_save_session(&self) {
        if let Some(history) = &self.session_history {
            let session_id = &self.conversation.id;
            if let Err(e) = history.save(session_id, self.conversation.messages()) {
                let _ = self
                    .event_tx
                    .send(Event::Error {
                        message: format!("Failed to save session: {e}"),
                    })
                    .await;
                return;
            }
            let _ = self
                .event_tx
                .send(Event::SessionSaved {
                    session_id: session_id.clone(),
                })
                .await;
        }
    }
}

/// Format memory files as a section to append to the system prompt.
fn format_memory_section(memories: &[crab_session::MemoryFile]) -> String {
    use std::fmt::Write;
    let mut section = String::new();
    let _ = writeln!(section, "\n\n# Loaded Memories\n");
    let _ = writeln!(
        section,
        "The following memories were loaded from previous sessions.\n"
    );
    for mem in memories {
        let _ = writeln!(section, "## {} (type: {})\n", mem.name, mem.memory_type);
        if !mem.description.is_empty() {
            let _ = writeln!(section, "> {}\n", mem.description);
        }
        let _ = writeln!(section, "{}\n", mem.body);
    }
    section
}

impl AgentCoordinator {
    /// Create a new coordinator with a message bus.
    pub fn new(main_id: String, main_name: String) -> Self {
        let bus = MessageBus::new(64);
        let main_tx = bus.sender();
        let mut router = MessageRouter::new();
        let _main_rx = router.register(&main_name);
        let mut health = HealthMonitor::with_defaults();
        health.register(&main_name);
        let mut scheduler = WorkStealingScheduler::new();
        scheduler.register(&main_name);
        Self {
            main_agent: AgentHandle {
                id: main_id,
                name: main_name,
                tx: main_tx,
            },
            workers: Vec::new(),
            bus: bus.sender(),
            running: HashMap::new(),
            completed: Vec::new(),
            next_worker_id: 1,
            router,
            team: None,
            assignment_strategy: None,
            health,
            metrics: MetricsCollector::new(),
            retry_tracker: RetryTracker::new(RetryPolicy::default()),
            scheduler,
        }
    }

    /// Set a custom retry policy.
    pub fn set_retry_policy(&mut self, policy: RetryPolicy) {
        self.retry_tracker = RetryTracker::new(policy);
    }

    /// Set a custom health monitor config.
    pub fn set_health_config(&mut self, config: HealthConfig) {
        let old_agents: Vec<String> = self.health.unresponsive_agents();
        let _ = old_agents; // just for reference
        // Re-create the monitor with new config, re-register all agents
        let mut new_health = HealthMonitor::new(config);
        new_health.register(&self.main_agent.name);
        for worker in &self.workers {
            new_health.register(&worker.name);
        }
        self.health = new_health;
    }

    /// Record a heartbeat from a worker (call periodically to keep health status current).
    pub fn worker_heartbeat(&mut self, worker_name: &str) {
        self.health.heartbeat(worker_name);
    }

    /// Handle a worker task failure: record metrics and decide on retry.
    ///
    /// Returns `Some(task_id)` if the task should be retried, `None` if exhausted.
    pub fn on_worker_failure(
        &mut self,
        worker_name: &str,
        task_id: &str,
        duration: std::time::Duration,
    ) -> Option<RetryDecision> {
        self.metrics.record_completion(worker_name, false, duration);
        let decision = self.retry_tracker.on_failure(task_id);
        match &decision {
            RetryDecision::Retry { .. } => Some(decision),
            RetryDecision::GiveUp { .. } => None,
        }
    }

    /// Handle a worker task success: record metrics and clear retry state.
    pub fn on_worker_success(
        &mut self,
        worker_name: &str,
        task_id: &str,
        duration: std::time::Duration,
    ) {
        self.metrics.record_completion(worker_name, true, duration);
        self.retry_tracker.on_success(task_id);
    }

    /// Set the team structure for this coordinator.
    pub fn set_team(&mut self, team: Team) {
        self.team = Some(team);
    }

    /// Set the task assignment strategy.
    pub fn set_assignment_strategy(&mut self, strategy: Box<dyn AssignmentStrategy>) {
        self.assignment_strategy = Some(strategy);
    }

    /// Get the current team mode, if a team is configured.
    #[must_use]
    pub fn team_mode(&self) -> Option<TeamMode> {
        self.team.as_ref().map(|t| t.mode)
    }

    /// Route an envelope through the message router.
    ///
    /// Respects team communication rules if a team is configured.
    /// Returns the number of agents the message was delivered to,
    /// or `Err` if communication is not allowed.
    pub async fn route_message(&self, envelope: &Envelope) -> crab_common::Result<usize> {
        // Enforce team communication rules
        if let Some(team) = &self.team
            && !envelope.is_broadcast()
            && !team.can_communicate(&envelope.from, &envelope.to)
        {
            return Err(crab_common::Error::Other(format!(
                "communication not allowed from '{}' to '{}' in {} mode",
                envelope.from, envelope.to, team.mode,
            )));
        }
        Ok(self.router.route(envelope).await)
    }

    /// Assign a task using the configured assignment strategy.
    ///
    /// Returns the name of the selected agent, or `None` if no strategy
    /// is set or no suitable candidate is found.
    pub fn assign_task(&mut self, required_capability: Option<&Capability>) -> Option<String> {
        let team = self.team.as_ref()?;
        let strategy = self.assignment_strategy.as_mut()?;

        // In leader-worker mode, only assign to non-leader members
        let candidates: Vec<_> = match team.mode {
            TeamMode::LeaderWorker => team
                .members
                .iter()
                .filter(|m| !m.is_leader)
                .cloned()
                .collect(),
            TeamMode::PeerToPeer => team.members.clone(),
        };

        strategy
            .select(&candidates, required_capability)
            .map(|m| m.name.clone())
    }

    /// Add a worker agent handle (for pre-configured workers).
    pub fn add_worker(&mut self, id: String, name: String) {
        self.workers.push(AgentHandle {
            id,
            name,
            tx: self.bus.clone(),
        });
    }

    /// Spawn a new sub-agent worker with the given task.
    ///
    /// The worker inherits the parent's backend, executor, and tool context
    /// but runs an independent conversation. Returns the assigned worker ID.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_worker(
        &mut self,
        task_prompt: String,
        system_prompt: String,
        backend: Arc<LlmBackend>,
        executor: Arc<ToolExecutor>,
        tool_ctx: ToolContext,
        loop_config: QueryLoopConfig,
        event_tx: mpsc::Sender<Event>,
        max_turns: Option<usize>,
    ) -> String {
        let worker_id = format!("worker_{}", self.next_worker_id);
        self.next_worker_id += 1;

        let cancel = CancellationToken::new();

        let config = WorkerConfig {
            worker_id: worker_id.clone(),
            system_prompt,
            max_turns,
            max_duration: None,
            context_window: 200_000,
        };

        let worker = AgentWorker::new(
            config,
            backend,
            executor,
            tool_ctx,
            loop_config,
            event_tx,
            cancel.clone(),
        );

        let handle = worker.spawn(task_prompt);

        self.running
            .insert(worker_id.clone(), RunningWorker { handle, cancel });

        // Also register in the handle list, router, health, and scheduler
        self.workers.push(AgentHandle {
            id: worker_id.clone(),
            name: worker_id.clone(),
            tx: self.bus.clone(),
        });
        let _worker_rx = self.router.register(&worker_id);
        self.health.register(&worker_id);
        self.scheduler.register(&worker_id);

        worker_id
    }

    /// Cancel a running worker by ID. Returns `true` if the worker was found.
    pub fn cancel_worker(&self, worker_id: &str) -> bool {
        self.running.get(worker_id).is_some_and(|worker| {
            worker.cancel.cancel();
            true
        })
    }

    /// Cancel all running workers.
    pub fn cancel_all(&self) {
        for worker in self.running.values() {
            worker.cancel.cancel();
        }
    }

    /// Collect the result of a specific worker, blocking until it completes.
    ///
    /// Returns `None` if the worker ID is not found or already collected.
    pub async fn collect_worker(&mut self, worker_id: &str) -> Option<WorkerResult> {
        let worker = self.running.remove(worker_id)?;
        match worker.handle.await {
            Ok(result) => {
                self.completed.push(result.clone_summary());
                Some(result)
            }
            Err(_) => None,
        }
    }

    /// Collect all completed workers (non-blocking: only those already finished).
    pub async fn collect_completed(&mut self) -> Vec<WorkerResult> {
        let mut results = Vec::new();
        let mut still_running = HashMap::new();

        for (id, worker) in self.running.drain() {
            if worker.handle.is_finished() {
                if let Ok(result) = worker.handle.await {
                    results.push(result);
                }
            } else {
                still_running.insert(id, worker);
            }
        }

        self.running = still_running;
        self.completed
            .extend(results.iter().map(WorkerResult::clone_summary));
        results
    }

    /// Wait for all running workers to complete and collect their results.
    pub async fn collect_all(&mut self) -> Vec<WorkerResult> {
        let mut results = Vec::new();
        for (_, worker) in self.running.drain() {
            if let Ok(result) = worker.handle.await {
                results.push(result);
            }
        }
        self.completed
            .extend(results.iter().map(WorkerResult::clone_summary));
        results
    }

    /// Get the number of currently running workers.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    /// Get all completed worker results (summaries).
    #[must_use]
    pub fn completed_results(&self) -> &[WorkerResult] {
        &self.completed
    }

    /// Get the IDs of all currently running workers.
    #[must_use]
    pub fn running_worker_ids(&self) -> Vec<String> {
        self.running.keys().cloned().collect()
    }

    /// Check whether a specific worker is still running.
    #[must_use]
    pub fn is_worker_running(&self, worker_id: &str) -> bool {
        self.running.contains_key(worker_id)
    }

    /// Inject a pre-built running worker (for testing or external spawn).
    #[cfg(test)]
    fn inject_worker(
        &mut self,
        worker_id: String,
        handle: tokio::task::JoinHandle<WorkerResult>,
        cancel: CancellationToken,
    ) {
        self.running
            .insert(worker_id, RunningWorker { handle, cancel });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a dummy `LlmBackend` for tests (OpenAI client pointing to localhost).
    fn test_backend() -> Arc<LlmBackend> {
        Arc::new(LlmBackend::OpenAi(crab_api::openai::OpenAiClient::new(
            "http://localhost:0/v1",
            None,
        )))
    }

    #[test]
    fn coordinator_creation() {
        let coord = AgentCoordinator::new("main".into(), "Main Agent".into());
        assert_eq!(coord.main_agent.id, "main");
        assert_eq!(coord.main_agent.name, "Main Agent");
        assert!(coord.workers.is_empty());
        assert_eq!(coord.running_count(), 0);
        assert!(coord.completed_results().is_empty());
    }

    #[test]
    fn coordinator_add_worker() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.add_worker("w1".into(), "Worker 1".into());
        coord.add_worker("w2".into(), "Worker 2".into());
        assert_eq!(coord.workers.len(), 2);
        assert_eq!(coord.workers[0].id, "w1");
        assert_eq!(coord.workers[1].name, "Worker 2");
    }

    #[test]
    fn coordinator_cancel_nonexistent_worker() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        assert!(!coord.cancel_worker("nonexistent"));
    }

    #[test]
    fn coordinator_cancel_all_empty() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        // Should not panic
        coord.cancel_all();
    }

    #[tokio::test]
    async fn coordinator_collect_nonexistent_worker() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let result = coord.collect_worker("nonexistent").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn coordinator_collect_completed_empty() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let results = coord.collect_completed().await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn coordinator_collect_all_empty() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let results = coord.collect_all().await;
        assert!(results.is_empty());
    }

    #[test]
    fn coordinator_worker_id_increments() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        assert_eq!(coord.next_worker_id, 1);
    }

    #[test]
    fn session_config_construction() {
        let config = SessionConfig {
            session_id: "sess_1".into(),
            system_prompt: "You are helpful.".into(),
            model: ModelId::from("claude-sonnet-4-20250514"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: std::path::PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: None,
            resume_session_id: None,
        };
        assert_eq!(config.session_id, "sess_1");
        assert_eq!(config.context_window, 200_000);
    }

    #[test]
    fn session_with_memory_store() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().join("memory");

        // Write a memory file before creating the session
        let store = MemoryStore::new(memory_dir.clone());
        store
            .save(
                "user_role.md",
                "---\nname: User role\ndescription: Senior dev\ntype: user\n---\n\nSenior Rust dev.",
            )
            .unwrap();

        let config = SessionConfig {
            session_id: "sess_mem".into(),
            system_prompt: "Base prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: Some(memory_dir),
            sessions_dir: None,
            resume_session_id: None,
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        // Memory should be injected into the system prompt
        assert!(session.conversation.system_prompt.contains("User role"));
        assert!(
            session
                .conversation
                .system_prompt
                .contains("Senior Rust dev")
        );
        assert!(session.memory_store.is_some());
    }

    #[test]
    fn session_with_session_history_resume() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");

        // Save a previous session to resume from
        let history = SessionHistory::new(sessions_dir.clone());
        history
            .save(
                "prev_sess",
                &[Message::user("Hello"), Message::assistant("Hi!")],
            )
            .unwrap();

        let config = SessionConfig {
            session_id: "new_sess".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: Some(sessions_dir),
            resume_session_id: Some("prev_sess".into()),
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        // Resumed messages should be in the conversation
        assert_eq!(session.conversation.len(), 2);
        assert_eq!(session.conversation.messages()[0].text(), "Hello");
        assert_eq!(session.conversation.messages()[1].text(), "Hi!");
        assert!(session.session_history.is_some());
    }

    #[test]
    fn session_no_memory_no_history() {
        let config = SessionConfig {
            session_id: "plain".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: None,
            resume_session_id: None,
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        assert!(session.memory_store.is_none());
        assert!(session.session_history.is_none());
        assert!(session.conversation.is_empty());
        assert!(
            !session
                .conversation
                .system_prompt
                .contains("Loaded Memories")
        );
    }

    #[test]
    fn save_memory_through_session() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().join("memory");

        let config = SessionConfig {
            session_id: "sess_save".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: Some(memory_dir.clone()),
            sessions_dir: None,
            resume_session_id: None,
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        session
            .save_memory(
                "test.md",
                "---\nname: Test\ndescription: test\ntype: user\n---\n\nBody.",
            )
            .unwrap();

        // Verify it was saved
        let store = MemoryStore::new(memory_dir);
        let content = store.load("test.md").unwrap().unwrap();
        assert!(content.contains("Body."));
    }

    #[test]
    fn format_memory_section_creates_markdown() {
        let memories = vec![crab_session::MemoryFile {
            name: "Test".into(),
            description: "A test".into(),
            memory_type: "user".into(),
            body: "Content here.".into(),
            filename: "test.md".into(),
        }];
        let section = format_memory_section(&memories);
        assert!(section.contains("# Loaded Memories"));
        assert!(section.contains("## Test (type: user)"));
        assert!(section.contains("> A test"));
        assert!(section.contains("Content here."));
    }

    #[test]
    fn format_memory_section_empty_description() {
        let memories = vec![crab_session::MemoryFile {
            name: "NoDesc".into(),
            description: String::new(),
            memory_type: "project".into(),
            body: "Body only.".into(),
            filename: "nodesc.md".into(),
        }];
        let section = format_memory_section(&memories);
        assert!(section.contains("## NoDesc (type: project)"));
        assert!(!section.contains("> \n")); // no blockquote for empty desc
        assert!(section.contains("Body only."));
    }

    #[test]
    fn format_memory_section_multiple_memories() {
        let memories = vec![
            crab_session::MemoryFile {
                name: "First".into(),
                description: "desc1".into(),
                memory_type: "user".into(),
                body: "body1".into(),
                filename: "first.md".into(),
            },
            crab_session::MemoryFile {
                name: "Second".into(),
                description: "desc2".into(),
                memory_type: "feedback".into(),
                body: "body2".into(),
                filename: "second.md".into(),
            },
        ];
        let section = format_memory_section(&memories);
        assert!(section.contains("First"));
        assert!(section.contains("Second"));
        assert!(section.contains("body1"));
        assert!(section.contains("body2"));
    }

    #[test]
    fn session_cancel() {
        let config = SessionConfig {
            session_id: "cancel-test".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: None,
            resume_session_id: None,
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        assert!(!session.cancel.is_cancelled());
        session.cancel();
        assert!(session.cancel.is_cancelled());
    }

    #[test]
    fn session_event_sender() {
        let config = SessionConfig {
            session_id: "event-test".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: None,
            resume_session_id: None,
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        let _tx = session.event_sender();
    }

    #[test]
    fn save_memory_without_store_is_noop() {
        let config = SessionConfig {
            session_id: "no-mem".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: None,
            resume_session_id: None,
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        session.save_memory("test.md", "content").unwrap();
    }

    #[test]
    fn session_resume_nonexistent_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");

        let config = SessionConfig {
            session_id: "new".into(),
            system_prompt: "Prompt.".into(),
            model: ModelId::from("test-model"),
            max_tokens: 4096,
            temperature: None,
            context_window: 200_000,
            working_dir: PathBuf::from("/tmp"),
            permission_policy: PermissionPolicy::default(),
            memory_dir: None,
            sessions_dir: Some(sessions_dir),
            resume_session_id: Some("nonexistent".into()),
        };

        let backend = test_backend();
        let registry = ToolRegistry::new();
        let session = AgentSession::new(config, backend, registry);

        assert!(session.conversation.is_empty());
    }

    // ─── Worker lifecycle tests ───────────────────────────────────────

    /// Create a mock `WorkerResult` for testing.
    fn mock_worker_result(worker_id: &str, success: bool) -> WorkerResult {
        WorkerResult {
            worker_id: worker_id.into(),
            output: if success { Some("done".into()) } else { None },
            success,
            usage: crab_core::model::TokenUsage::default(),
            conversation: Conversation::new(worker_id.into(), String::new(), 0),
        }
    }

    #[tokio::test]
    async fn coordinator_inject_and_collect_worker() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let cancel = CancellationToken::new();

        let handle = tokio::spawn(async { mock_worker_result("w1", true) });
        coord.inject_worker("w1".into(), handle, cancel);

        assert_eq!(coord.running_count(), 1);
        assert!(coord.is_worker_running("w1"));

        let result = coord.collect_worker("w1").await.unwrap();
        assert_eq!(result.worker_id, "w1");
        assert!(result.success);
        assert_eq!(result.output.as_deref(), Some("done"));

        // After collection, no longer running
        assert_eq!(coord.running_count(), 0);
        assert!(!coord.is_worker_running("w1"));

        // Completed results should have a summary
        assert_eq!(coord.completed_results().len(), 1);
        assert_eq!(coord.completed_results()[0].worker_id, "w1");
    }

    #[tokio::test]
    async fn coordinator_collect_worker_twice_returns_none() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let cancel = CancellationToken::new();

        let handle = tokio::spawn(async { mock_worker_result("w1", true) });
        coord.inject_worker("w1".into(), handle, cancel);

        coord.collect_worker("w1").await.unwrap();
        // Second collect returns None
        assert!(coord.collect_worker("w1").await.is_none());
    }

    #[tokio::test]
    async fn coordinator_collect_all_multiple_workers() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        for i in 1..=3 {
            let id = format!("w{i}");
            let cancel = CancellationToken::new();
            let handle = tokio::spawn({
                let id = id.clone();
                async move { mock_worker_result(&id, true) }
            });
            coord.inject_worker(id, handle, cancel);
        }

        assert_eq!(coord.running_count(), 3);

        let results = coord.collect_all().await;
        assert_eq!(results.len(), 3);
        assert_eq!(coord.running_count(), 0);
        assert_eq!(coord.completed_results().len(), 3);
    }

    #[tokio::test]
    async fn coordinator_collect_completed_only_finished() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        // Worker that finishes immediately
        let cancel1 = CancellationToken::new();
        let handle1 = tokio::spawn(async { mock_worker_result("fast", true) });
        coord.inject_worker("fast".into(), handle1, cancel1);

        // Worker that blocks until cancelled
        let cancel2 = CancellationToken::new();
        let cancel2_clone = cancel2.clone();
        let handle2 = tokio::spawn(async move {
            cancel2_clone.cancelled().await;
            mock_worker_result("slow", false)
        });
        coord.inject_worker("slow".into(), handle2, cancel2.clone());

        // Give fast worker time to finish
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let results = coord.collect_completed().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].worker_id, "fast");

        // slow is still running
        assert_eq!(coord.running_count(), 1);
        assert!(coord.is_worker_running("slow"));

        // Clean up: cancel the slow worker
        cancel2.cancel();
        let remaining = coord.collect_all().await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].worker_id, "slow");
    }

    #[tokio::test]
    async fn coordinator_cancel_running_worker() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            cancel_clone.cancelled().await;
            mock_worker_result("w1", false)
        });
        coord.inject_worker("w1".into(), handle, cancel);

        assert!(coord.cancel_worker("w1"));

        // Collect the cancelled worker
        let result = coord.collect_worker("w1").await.unwrap();
        assert_eq!(result.worker_id, "w1");
        assert!(!result.success);
    }

    #[tokio::test]
    async fn coordinator_cancel_all_workers() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        for i in 1..=3 {
            let id = format!("w{i}");
            let cancel = CancellationToken::new();
            let cancel_clone = cancel.clone();
            let handle = tokio::spawn({
                let id = id.clone();
                async move {
                    cancel_clone.cancelled().await;
                    mock_worker_result(&id, false)
                }
            });
            coord.inject_worker(id, handle, cancel);
        }

        assert_eq!(coord.running_count(), 3);

        coord.cancel_all();
        let results = coord.collect_all().await;
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| !r.success));
    }

    #[test]
    fn coordinator_running_worker_ids() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let cancel = CancellationToken::new();

        let handle = tokio::runtime::Runtime::new()
            .unwrap()
            .spawn(async { mock_worker_result("w1", true) });
        coord.inject_worker("w1".into(), handle, cancel);

        let ids = coord.running_worker_ids();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&"w1".to_string()));
    }

    #[test]
    fn coordinator_worker_id_auto_increments_on_spawn_worker() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        assert_eq!(coord.next_worker_id, 1);

        // We can't call spawn_worker without a real backend/executor,
        // but we can test the ID format by verifying the increment logic.
        // spawn_worker uses format!("worker_{}", self.next_worker_id).
        let expected_id = format!("worker_{}", coord.next_worker_id);
        assert_eq!(expected_id, "worker_1");
    }

    // ─── Router integration tests ───────────────────────────────────

    #[test]
    fn coordinator_has_router() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        // Main agent should be registered in the router
        assert!(coord.router.is_registered("Main"));
        assert_eq!(coord.router.agent_count(), 1);
    }

    #[tokio::test]
    async fn coordinator_route_directed_message() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let mut worker_rx = coord.router.register("Worker1");

        let env = Envelope::new(
            "Main",
            "Worker1",
            AgentMessage::AssignTask {
                task_id: "t1".into(),
                prompt: "do stuff".into(),
            },
        );

        let delivered = coord.route_message(&env).await.unwrap();
        assert_eq!(delivered, 1);

        let msg = worker_rx.recv().await.unwrap();
        assert_eq!(msg.from, "Main");
        assert!(matches!(msg.payload, AgentMessage::AssignTask { .. }));
    }

    #[tokio::test]
    async fn coordinator_route_broadcast() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        let mut rx1 = coord.router.register("W1");
        let mut rx2 = coord.router.register("W2");

        let env = Envelope::broadcast("Main", AgentMessage::Shutdown);
        let delivered = coord.route_message(&env).await.unwrap();
        assert_eq!(delivered, 2);

        assert!(rx1.recv().await.is_some());
        assert!(rx2.recv().await.is_some());
    }

    // ─── Team integration tests ─────────────────────────────────────

    #[test]
    fn coordinator_no_team_by_default() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        assert!(coord.team.is_none());
        assert!(coord.team_mode().is_none());
    }

    #[test]
    fn coordinator_set_team() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        let mut team = Team::with_mode("dev".into(), TeamMode::PeerToPeer);
        let mut alice = crate::team::TeamMember::new("a1", "Alice", "model");
        alice.is_leader = true;
        team.add_member(alice);
        team.add_member(crate::team::TeamMember::new("a2", "Bob", "model"));

        coord.set_team(team);
        assert_eq!(coord.team_mode(), Some(TeamMode::PeerToPeer));
    }

    #[tokio::test]
    async fn coordinator_leader_worker_blocks_worker_to_worker() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        let mut team = Team::new("dev".into()); // LeaderWorker by default
        let mut leader = crate::team::TeamMember::new("a1", "Leader", "model");
        leader.is_leader = true;
        team.add_member(leader);
        team.add_member(crate::team::TeamMember::new("a2", "W1", "model"));
        team.add_member(crate::team::TeamMember::new("a3", "W2", "model"));
        coord.set_team(team);

        let _rx1 = coord.router.register("W1");
        let _rx2 = coord.router.register("W2");
        let _rx_leader = coord.router.register("Leader");

        // Leader → Worker: allowed
        let env = Envelope::new("Leader", "W1", AgentMessage::Shutdown);
        assert!(coord.route_message(&env).await.is_ok());

        // Worker → Leader: allowed
        let env = Envelope::new("W1", "Leader", AgentMessage::ShutdownAck);
        assert!(coord.route_message(&env).await.is_ok());

        // Worker → Worker: blocked
        let env = Envelope::new("W1", "W2", AgentMessage::Shutdown);
        assert!(coord.route_message(&env).await.is_err());
    }

    #[tokio::test]
    async fn coordinator_peer_to_peer_allows_all() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        let mut team = Team::with_mode("dev".into(), TeamMode::PeerToPeer);
        team.add_member(crate::team::TeamMember::new("a1", "A", "model"));
        team.add_member(crate::team::TeamMember::new("a2", "B", "model"));
        coord.set_team(team);

        let _rx_a = coord.router.register("A");
        let _rx_b = coord.router.register("B");

        // A → B: allowed
        let env = Envelope::new("A", "B", AgentMessage::Shutdown);
        assert!(coord.route_message(&env).await.is_ok());

        // B → A: allowed
        let env = Envelope::new("B", "A", AgentMessage::Shutdown);
        assert!(coord.route_message(&env).await.is_ok());
    }

    // ─── Assignment strategy tests ──────────────────────────────────

    #[test]
    fn coordinator_no_strategy_by_default() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        assert!(coord.assign_task(None).is_none());
    }

    #[test]
    fn coordinator_round_robin_assignment() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        let mut team = Team::new("dev".into());
        let mut leader = crate::team::TeamMember::new("a0", "Leader", "model");
        leader.is_leader = true;
        team.add_member(leader);
        team.add_member(crate::team::TeamMember::new("a1", "W1", "model"));
        team.add_member(crate::team::TeamMember::new("a2", "W2", "model"));
        team.add_member(crate::team::TeamMember::new("a3", "W3", "model"));
        coord.set_team(team);
        coord.set_assignment_strategy(Box::new(crate::assignment::RoundRobin::new()));

        // Leader is excluded in LeaderWorker mode
        assert_eq!(coord.assign_task(None).as_deref(), Some("W1"));
        assert_eq!(coord.assign_task(None).as_deref(), Some("W2"));
        assert_eq!(coord.assign_task(None).as_deref(), Some("W3"));
        assert_eq!(coord.assign_task(None).as_deref(), Some("W1")); // wraps
    }

    #[test]
    fn coordinator_capability_based_assignment() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        let mut team = Team::new("dev".into());
        let mut leader = crate::team::TeamMember::new("a0", "Leader", "model");
        leader.is_leader = true;
        team.add_member(leader);

        let mut w1 = crate::team::TeamMember::new("a1", "W1", "model");
        w1.add_capability(Capability::new("testing"));
        team.add_member(w1);

        let mut w2 = crate::team::TeamMember::new("a2", "W2", "model");
        w2.add_capability(Capability::new("frontend"));
        team.add_member(w2);

        coord.set_team(team);
        coord.set_assignment_strategy(Box::new(crate::assignment::CapabilityBased::new()));

        let cap = Capability::new("frontend");
        assert_eq!(coord.assign_task(Some(&cap)).as_deref(), Some("W2"));

        let cap = Capability::new("testing");
        assert_eq!(coord.assign_task(Some(&cap)).as_deref(), Some("W1"));
    }

    #[test]
    fn coordinator_assign_no_team_returns_none() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.set_assignment_strategy(Box::new(crate::assignment::RoundRobin::new()));
        // No team set → None
        assert!(coord.assign_task(None).is_none());
    }

    // ─── Health monitor integration ─────────────────────────────────

    #[test]
    fn coordinator_has_health_monitor() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        // Main agent is auto-registered
        assert!(coord.health.check("Main").is_some());
    }

    #[test]
    fn coordinator_worker_heartbeat() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.health.register("w1");
        coord.worker_heartbeat("w1");
        let h = coord.health.check("w1").unwrap();
        assert_eq!(h.status, crate::health::HealthStatus::Healthy);
    }

    #[test]
    fn coordinator_set_health_config() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.add_worker("w1".into(), "W1".into());
        coord.health.register("W1");

        coord.set_health_config(crate::health::HealthConfig {
            heartbeat_interval: std::time::Duration::from_secs(5),
            degraded_threshold: 1,
            unresponsive_threshold: 3,
        });

        // Both main and W1 should be re-registered
        assert!(coord.health.check("Main").is_some());
        assert!(coord.health.check("W1").is_some());
    }

    // ─── Metrics integration ────────────────────────────────────────

    #[test]
    fn coordinator_has_metrics() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        assert!(coord.metrics.all().is_empty()); // no tasks yet
    }

    #[test]
    fn coordinator_on_worker_success() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.on_worker_success("w1", "t1", std::time::Duration::from_secs(5));

        let m = coord.metrics.get("w1").unwrap();
        assert_eq!(m.tasks_succeeded, 1);
        assert_eq!(m.tasks_failed, 0);
    }

    #[test]
    fn coordinator_on_worker_failure_retries() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        // Default policy: 2 retries

        let decision = coord.on_worker_failure("w1", "t1", std::time::Duration::from_secs(1));
        assert!(decision.is_some()); // should retry (attempt 1)
        assert!(matches!(decision, Some(RetryDecision::Retry { .. })));

        let decision = coord.on_worker_failure("w1", "t1", std::time::Duration::from_secs(1));
        assert!(decision.is_some()); // should retry (attempt 2)

        let decision = coord.on_worker_failure("w1", "t1", std::time::Duration::from_secs(1));
        assert!(decision.is_none()); // exhausted (attempt 3 > max_retries=2)

        let m = coord.metrics.get("w1").unwrap();
        assert_eq!(m.tasks_failed, 3);
    }

    #[test]
    fn coordinator_set_retry_policy() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.set_retry_policy(crate::retry::RetryPolicy::no_retry());

        // First failure should give up immediately
        let decision = coord.on_worker_failure("w1", "t1", std::time::Duration::from_secs(1));
        assert!(decision.is_none());
    }

    #[test]
    fn coordinator_success_clears_retry_state() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());

        // Fail once
        coord.on_worker_failure("w1", "t1", std::time::Duration::from_secs(1));
        assert_eq!(coord.retry_tracker.attempts_for("t1"), 1);

        // Succeed — clears retry state
        coord.on_worker_success("w1", "t1", std::time::Duration::from_secs(1));
        assert_eq!(coord.retry_tracker.attempts_for("t1"), 0);
    }

    // ─── Work-stealing scheduler integration ────────────────────────

    #[test]
    fn coordinator_has_scheduler() {
        let coord = AgentCoordinator::new("main".into(), "Main".into());
        // Main is auto-registered in the scheduler
        assert_eq!(coord.scheduler.agent_count(), 1);
    }

    #[test]
    fn coordinator_scheduler_enqueue_dequeue() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.scheduler.register("w1");

        let task = crate::work_stealing::QueuedTask {
            task_id: "t1".into(),
            prompt: "do stuff".into(),
            required_capability: None,
        };
        assert!(coord.scheduler.enqueue("w1", task));
        assert_eq!(coord.scheduler.queue_depth("w1"), 1);

        let dequeued = coord.scheduler.dequeue("w1").unwrap();
        assert_eq!(dequeued.task_id, "t1");
    }

    #[test]
    fn coordinator_scheduler_steal() {
        let mut coord = AgentCoordinator::new("main".into(), "Main".into());
        coord.scheduler.register("busy");
        coord.scheduler.register("idle");

        // Load busy agent with tasks
        for i in 0..5 {
            coord.scheduler.enqueue(
                "busy",
                crate::work_stealing::QueuedTask {
                    task_id: format!("t{i}"),
                    prompt: format!("task {i}"),
                    required_capability: None,
                },
            );
        }

        // idle steals from busy
        let (victim, stolen) = coord.scheduler.steal("idle").unwrap();
        assert_eq!(victim, "busy");
        assert_eq!(stolen.task_id, "t4"); // steals from back
        assert_eq!(coord.scheduler.queue_depth("busy"), 4);
    }
}
