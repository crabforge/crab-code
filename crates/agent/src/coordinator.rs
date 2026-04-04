use std::sync::Arc;

use crab_api::LlmBackend;
use crab_core::event::Event;
use crab_core::message::Message;
use crab_core::model::ModelId;
use crab_core::permission::PermissionPolicy;
use crab_core::tool::ToolContext;
use crab_session::Conversation;
use crab_tools::executor::ToolExecutor;
use crab_tools::registry::ToolRegistry;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::message_bus::{AgentMessage, MessageBus};
use crate::query_loop::{self, QueryLoopConfig};

/// Multi-agent orchestrator. Manages the main agent and worker pool.
pub struct AgentCoordinator {
    pub main_agent: AgentHandle,
    pub workers: Vec<AgentHandle>,
    pub bus: mpsc::Sender<AgentMessage>,
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
}

impl AgentSession {
    /// Initialize a new agent session.
    pub fn new(
        session_config: SessionConfig,
        backend: Arc<LlmBackend>,
        registry: ToolRegistry,
    ) -> Self {
        let conversation = Conversation::new(
            session_config.session_id.clone(),
            session_config.system_prompt,
            session_config.context_window,
        );

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
        }
    }

    /// Handle user input: add user message and run the query loop.
    pub async fn handle_user_input(
        &mut self,
        input: &str,
    ) -> crab_common::Result<()> {
        self.conversation.push(Message::user(input));

        query_loop::query_loop(
            &mut self.conversation,
            &self.backend,
            &self.executor,
            &self.tool_ctx,
            &self.config,
            self.event_tx.clone(),
            self.cancel.clone(),
        )
        .await
    }

    /// Cancel the running query loop.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Get a clone of the event sender for external use.
    pub fn event_sender(&self) -> mpsc::Sender<Event> {
        self.event_tx.clone()
    }
}

impl AgentCoordinator {
    /// Create a new coordinator with a message bus.
    pub fn new(main_id: String, main_name: String) -> Self {
        let bus = MessageBus::new(64);
        let main_tx = bus.sender();
        Self {
            main_agent: AgentHandle {
                id: main_id,
                name: main_name,
                tx: main_tx,
            },
            workers: Vec::new(),
            bus: bus.sender(),
        }
    }

    /// Add a worker agent.
    pub fn add_worker(&mut self, id: String, name: String) {
        self.workers.push(AgentHandle {
            id,
            name,
            tx: self.bus.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_creation() {
        let coord = AgentCoordinator::new("main".into(), "Main Agent".into());
        assert_eq!(coord.main_agent.id, "main");
        assert_eq!(coord.main_agent.name, "Main Agent");
        assert!(coord.workers.is_empty());
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
        };
        assert_eq!(config.session_id, "sess_1");
        assert_eq!(config.context_window, 200_000);
    }
}
