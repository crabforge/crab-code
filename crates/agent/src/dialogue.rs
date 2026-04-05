//! Multi-turn dialogue management: state machine, turn planning, and
//! dialogue policy for controlling conversation flow.

use std::fmt::Write;

// ── Conversation state machine ────────────────────────────────────────

/// State of the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationState {
    /// Waiting for user input.
    Idle,
    /// Sending a query to the LLM.
    Querying,
    /// Executing tool calls from the LLM response.
    ToolExecution,
    /// Waiting for user to respond (e.g., permission prompt, clarification).
    WaitingUser,
    /// Summarizing the conversation (compaction).
    Summarizing,
    /// Conversation has ended.
    Finished,
}

impl std::fmt::Display for ConversationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Querying => write!(f, "querying"),
            Self::ToolExecution => write!(f, "tool_execution"),
            Self::WaitingUser => write!(f, "waiting_user"),
            Self::Summarizing => write!(f, "summarizing"),
            Self::Finished => write!(f, "finished"),
        }
    }
}

/// Events that trigger state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogueEvent {
    /// User submitted a message.
    UserMessage,
    /// LLM query was sent.
    QuerySent,
    /// LLM responded with text only (no tool calls).
    TextResponse,
    /// LLM responded with tool calls.
    ToolCallResponse,
    /// Tool execution completed.
    ToolExecutionDone,
    /// User confirmation/input received.
    UserConfirmation,
    /// Compaction/summarization triggered.
    SummarizeTriggered,
    /// Summarization completed.
    SummarizeDone,
    /// Conversation ended (user quit or max turns).
    End,
    /// An error occurred.
    Error,
}

impl std::fmt::Display for DialogueEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserMessage => write!(f, "user_message"),
            Self::QuerySent => write!(f, "query_sent"),
            Self::TextResponse => write!(f, "text_response"),
            Self::ToolCallResponse => write!(f, "tool_call_response"),
            Self::ToolExecutionDone => write!(f, "tool_execution_done"),
            Self::UserConfirmation => write!(f, "user_confirmation"),
            Self::SummarizeTriggered => write!(f, "summarize_triggered"),
            Self::SummarizeDone => write!(f, "summarize_done"),
            Self::End => write!(f, "end"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Result of a state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionResult {
    pub from: ConversationState,
    pub to: ConversationState,
    pub event: DialogueEvent,
    pub valid: bool,
}

/// Conversation state machine with transition rules.
#[derive(Debug)]
pub struct ConversationStateMachine {
    state: ConversationState,
    transition_count: usize,
    history: Vec<TransitionResult>,
    /// Maximum history entries to keep.
    max_history: usize,
}

impl ConversationStateMachine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: ConversationState::Idle,
            transition_count: 0,
            history: Vec::new(),
            max_history: 100,
        }
    }

    /// Get the current state.
    #[must_use]
    pub fn state(&self) -> ConversationState {
        self.state
    }

    /// Get the total number of transitions.
    #[must_use]
    pub fn transition_count(&self) -> usize {
        self.transition_count
    }

    /// Get the transition history.
    #[must_use]
    pub fn history(&self) -> &[TransitionResult] {
        &self.history
    }

    /// Process an event and transition to the next state.
    ///
    /// Returns the transition result. Invalid transitions are recorded but
    /// do not change the state.
    pub fn on_event(&mut self, event: DialogueEvent) -> TransitionResult {
        let from = self.state;
        let to = next_state(from, event);
        let valid = to.is_some();
        let new_state = to.unwrap_or(from);

        let result = TransitionResult {
            from,
            to: new_state,
            event,
            valid,
        };

        if valid {
            self.state = new_state;
            self.transition_count += 1;
        }

        if self.history.len() < self.max_history {
            self.history.push(result.clone());
        }

        result
    }

    /// Check if a transition is valid without performing it.
    #[must_use]
    pub fn can_transition(&self, event: DialogueEvent) -> bool {
        next_state(self.state, event).is_some()
    }

    /// Reset to idle state.
    pub fn reset(&mut self) {
        self.state = ConversationState::Idle;
    }

    /// Check if the conversation has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state == ConversationState::Finished
    }
}

impl Default for ConversationStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the next state for a given (state, event) pair.
/// Returns `None` if the transition is invalid.
fn next_state(state: ConversationState, event: DialogueEvent) -> Option<ConversationState> {
    use ConversationState as S;
    use DialogueEvent as E;

    match (state, event) {
        // From Idle
        (S::Idle, E::UserMessage) => Some(S::Querying),
        (S::Idle, E::End) => Some(S::Finished),

        // From Querying
        (S::Querying, E::QuerySent) => Some(S::Querying), // still querying (streaming)
        (S::Querying, E::TextResponse) => Some(S::Idle),
        (S::Querying, E::ToolCallResponse) => Some(S::ToolExecution),
        (S::Querying, E::Error) => Some(S::Idle),

        // From ToolExecution
        (S::ToolExecution, E::ToolExecutionDone) => Some(S::Querying),
        (S::ToolExecution, E::UserConfirmation) => Some(S::WaitingUser),
        (S::ToolExecution, E::Error) => Some(S::Idle),

        // From WaitingUser
        (S::WaitingUser, E::UserConfirmation) => Some(S::ToolExecution),
        (S::WaitingUser, E::UserMessage) => Some(S::Querying),
        (S::WaitingUser, E::End) => Some(S::Finished),

        // From any state: summarize
        (_, E::SummarizeTriggered) if state != S::Finished && state != S::Summarizing => {
            Some(S::Summarizing)
        }

        // From Summarizing
        (S::Summarizing, E::SummarizeDone) => Some(S::Idle),
        (S::Summarizing, E::Error) => Some(S::Idle),

        // End from most states
        (_, E::End) if state != S::Finished => Some(S::Finished),

        _ => None,
    }
}

// ── Turn planner ──────────────────────────────────────────────────────

/// Planned action for the next turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedAction {
    /// Continue with tool execution (agent loop continues).
    ContinueToolLoop,
    /// Send the accumulated results to the LLM for the next response.
    SendToLlm,
    /// Request user input before proceeding.
    RequestUserInput { prompt: String },
    /// Summarize the conversation to free context.
    Summarize,
    /// End the conversation.
    Finish { reason: String },
}

impl std::fmt::Display for PlannedAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContinueToolLoop => write!(f, "continue_tool_loop"),
            Self::SendToLlm => write!(f, "send_to_llm"),
            Self::RequestUserInput { prompt } => write!(f, "request_input: {prompt}"),
            Self::Summarize => write!(f, "summarize"),
            Self::Finish { reason } => write!(f, "finish: {reason}"),
        }
    }
}

/// Context passed to the turn planner for decision-making.
#[derive(Debug, Clone)]
pub struct TurnContext {
    /// Current turn number (1-based).
    pub turn: usize,
    /// Number of consecutive tool calls in this turn.
    pub tool_calls_in_turn: usize,
    /// Whether the last tool call required user confirmation.
    pub last_needed_confirmation: bool,
    /// Estimated context window usage (0.0 to 1.0).
    pub context_usage: f64,
    /// Whether the LLM response contained a stop/end signal.
    pub has_stop_signal: bool,
    /// Number of errors in this turn.
    pub errors_in_turn: usize,
}

impl Default for TurnContext {
    fn default() -> Self {
        Self {
            turn: 1,
            tool_calls_in_turn: 0,
            last_needed_confirmation: false,
            context_usage: 0.0,
            has_stop_signal: false,
            errors_in_turn: 0,
        }
    }
}

/// Plans the next action based on conversation state and policy.
#[must_use]
pub fn plan_next_turn(ctx: &TurnContext, policy: &DialoguePolicy) -> PlannedAction {
    // Check if we should finish
    if ctx.has_stop_signal {
        return PlannedAction::Finish {
            reason: "LLM signaled completion.".into(),
        };
    }

    if ctx.turn > policy.max_turns {
        return PlannedAction::Finish {
            reason: format!("Maximum turns ({}) reached.", policy.max_turns),
        };
    }

    // Check if we need to summarize
    if ctx.context_usage >= policy.summarize_threshold {
        return PlannedAction::Summarize;
    }

    // Check if we should ask user for confirmation
    if ctx.tool_calls_in_turn >= policy.confirm_after_tools && policy.confirm_after_tools > 0 {
        return PlannedAction::RequestUserInput {
            prompt: format!(
                "Executed {} tool calls. Continue or provide new direction?",
                ctx.tool_calls_in_turn,
            ),
        };
    }

    // Check if too many errors
    if ctx.errors_in_turn >= policy.max_errors_per_turn {
        return PlannedAction::RequestUserInput {
            prompt: "Multiple errors encountered. Would you like to adjust the approach?".into(),
        };
    }

    // Check if we should confirm periodically
    if policy.confirm_every_n_turns > 0
        && ctx.turn > 0
        && ctx.turn % policy.confirm_every_n_turns == 0
    {
        return PlannedAction::RequestUserInput {
            prompt: format!("Turn {} complete. Continue?", ctx.turn),
        };
    }

    // Default: continue the tool loop or send to LLM
    if ctx.tool_calls_in_turn > 0 {
        PlannedAction::SendToLlm
    } else {
        PlannedAction::ContinueToolLoop
    }
}

// ── Dialogue policy ───────────────────────────────────────────────────

/// Policy controlling conversation flow and limits.
#[derive(Debug, Clone)]
pub struct DialoguePolicy {
    /// Maximum number of turns before auto-stopping.
    pub max_turns: usize,
    /// Context usage threshold (0.0-1.0) that triggers auto-summarization.
    pub summarize_threshold: f64,
    /// Request user confirmation after this many tool calls in a single turn.
    /// Set to 0 to disable.
    pub confirm_after_tools: usize,
    /// Request user confirmation every N turns. Set to 0 to disable.
    pub confirm_every_n_turns: usize,
    /// Maximum errors per turn before asking user.
    pub max_errors_per_turn: usize,
    /// Whether to auto-summarize when compaction threshold is hit.
    pub auto_summarize: bool,
    /// Whether to allow the agent to continue without user input
    /// (autonomous mode).
    pub autonomous: bool,
}

impl Default for DialoguePolicy {
    fn default() -> Self {
        Self {
            max_turns: 50,
            summarize_threshold: 0.8,
            confirm_after_tools: 10,
            confirm_every_n_turns: 0,
            max_errors_per_turn: 3,
            auto_summarize: true,
            autonomous: false,
        }
    }
}

impl DialoguePolicy {
    /// Create a strict policy that confirms frequently.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            max_turns: 20,
            summarize_threshold: 0.7,
            confirm_after_tools: 5,
            confirm_every_n_turns: 5,
            max_errors_per_turn: 2,
            auto_summarize: true,
            autonomous: false,
        }
    }

    /// Create a permissive policy for autonomous operation.
    #[must_use]
    pub fn autonomous() -> Self {
        Self {
            max_turns: 200,
            summarize_threshold: 0.85,
            confirm_after_tools: 0,
            confirm_every_n_turns: 0,
            max_errors_per_turn: 5,
            auto_summarize: true,
            autonomous: true,
        }
    }

    /// Check if the policy allows continuing without user input.
    #[must_use]
    pub fn allows_autonomous(&self) -> bool {
        self.autonomous
    }

    /// Check if a turn number exceeds the maximum.
    #[must_use]
    pub fn is_turn_limit_reached(&self, turn: usize) -> bool {
        turn > self.max_turns
    }

    /// Check if context usage requires summarization.
    #[must_use]
    pub fn needs_summarization(&self, context_usage: f64) -> bool {
        self.auto_summarize && context_usage >= self.summarize_threshold
    }

    /// Format the policy as a summary string.
    #[must_use]
    pub fn to_summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Dialogue Policy:");
        let _ = writeln!(out, "  Max turns: {}", self.max_turns);
        let _ = writeln!(out, "  Summarize at: {:.0}%", self.summarize_threshold * 100.0);
        let _ = writeln!(out, "  Confirm after tools: {}", self.confirm_after_tools);
        let _ = writeln!(out, "  Confirm every N turns: {}", self.confirm_every_n_turns);
        let _ = writeln!(out, "  Max errors/turn: {}", self.max_errors_per_turn);
        let _ = writeln!(out, "  Autonomous: {}", self.autonomous);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ConversationState ──────────────────────────────────────────

    #[test]
    fn state_display() {
        assert_eq!(ConversationState::Idle.to_string(), "idle");
        assert_eq!(ConversationState::Querying.to_string(), "querying");
        assert_eq!(ConversationState::ToolExecution.to_string(), "tool_execution");
        assert_eq!(ConversationState::WaitingUser.to_string(), "waiting_user");
        assert_eq!(ConversationState::Summarizing.to_string(), "summarizing");
        assert_eq!(ConversationState::Finished.to_string(), "finished");
    }

    // ── DialogueEvent ──────────────────────────────────────────────

    #[test]
    fn event_display() {
        assert_eq!(DialogueEvent::UserMessage.to_string(), "user_message");
        assert_eq!(DialogueEvent::ToolCallResponse.to_string(), "tool_call_response");
        assert_eq!(DialogueEvent::End.to_string(), "end");
    }

    // ── ConversationStateMachine ───────────────────────────────────

    #[test]
    fn sm_starts_idle() {
        let sm = ConversationStateMachine::new();
        assert_eq!(sm.state(), ConversationState::Idle);
        assert_eq!(sm.transition_count(), 0);
        assert!(!sm.is_finished());
    }

    #[test]
    fn sm_idle_to_querying_on_user_message() {
        let mut sm = ConversationStateMachine::new();
        let result = sm.on_event(DialogueEvent::UserMessage);
        assert!(result.valid);
        assert_eq!(result.from, ConversationState::Idle);
        assert_eq!(result.to, ConversationState::Querying);
        assert_eq!(sm.state(), ConversationState::Querying);
    }

    #[test]
    fn sm_querying_to_idle_on_text_response() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        let result = sm.on_event(DialogueEvent::TextResponse);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Idle);
    }

    #[test]
    fn sm_querying_to_tool_execution() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        let result = sm.on_event(DialogueEvent::ToolCallResponse);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::ToolExecution);
    }

    #[test]
    fn sm_tool_execution_to_querying() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::ToolCallResponse);
        let result = sm.on_event(DialogueEvent::ToolExecutionDone);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Querying);
    }

    #[test]
    fn sm_tool_execution_to_waiting_user() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::ToolCallResponse);
        let result = sm.on_event(DialogueEvent::UserConfirmation);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::WaitingUser);
    }

    #[test]
    fn sm_waiting_user_to_tool_execution() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::ToolCallResponse);
        sm.on_event(DialogueEvent::UserConfirmation);
        let result = sm.on_event(DialogueEvent::UserConfirmation);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::ToolExecution);
    }

    #[test]
    fn sm_waiting_user_to_querying_on_new_message() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::ToolCallResponse);
        sm.on_event(DialogueEvent::UserConfirmation);
        let result = sm.on_event(DialogueEvent::UserMessage);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Querying);
    }

    #[test]
    fn sm_summarize_from_idle() {
        let mut sm = ConversationStateMachine::new();
        let result = sm.on_event(DialogueEvent::SummarizeTriggered);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Summarizing);
    }

    #[test]
    fn sm_summarize_done_to_idle() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::SummarizeTriggered);
        let result = sm.on_event(DialogueEvent::SummarizeDone);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Idle);
    }

    #[test]
    fn sm_end_from_idle() {
        let mut sm = ConversationStateMachine::new();
        let result = sm.on_event(DialogueEvent::End);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Finished);
        assert!(sm.is_finished());
    }

    #[test]
    fn sm_end_from_querying() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        let result = sm.on_event(DialogueEvent::End);
        assert!(result.valid);
        assert!(sm.is_finished());
    }

    #[test]
    fn sm_error_from_querying_to_idle() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        let result = sm.on_event(DialogueEvent::Error);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Idle);
    }

    #[test]
    fn sm_error_from_tool_execution_to_idle() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::ToolCallResponse);
        let result = sm.on_event(DialogueEvent::Error);
        assert!(result.valid);
        assert_eq!(sm.state(), ConversationState::Idle);
    }

    #[test]
    fn sm_invalid_transition() {
        let mut sm = ConversationStateMachine::new();
        // Can't get TextResponse while Idle
        let result = sm.on_event(DialogueEvent::TextResponse);
        assert!(!result.valid);
        assert_eq!(sm.state(), ConversationState::Idle); // unchanged
    }

    #[test]
    fn sm_can_transition() {
        let sm = ConversationStateMachine::new();
        assert!(sm.can_transition(DialogueEvent::UserMessage));
        assert!(!sm.can_transition(DialogueEvent::TextResponse));
    }

    #[test]
    fn sm_transition_count() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::TextResponse);
        assert_eq!(sm.transition_count(), 2);
    }

    #[test]
    fn sm_history_recorded() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.on_event(DialogueEvent::TextResponse);
        assert_eq!(sm.history().len(), 2);
        assert_eq!(sm.history()[0].event, DialogueEvent::UserMessage);
    }

    #[test]
    fn sm_reset() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::UserMessage);
        sm.reset();
        assert_eq!(sm.state(), ConversationState::Idle);
    }

    #[test]
    fn sm_finished_blocks_end() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::End);
        // Already finished, End again should be invalid
        let result = sm.on_event(DialogueEvent::End);
        assert!(!result.valid);
    }

    #[test]
    fn sm_no_summarize_when_finished() {
        let mut sm = ConversationStateMachine::new();
        sm.on_event(DialogueEvent::End);
        let result = sm.on_event(DialogueEvent::SummarizeTriggered);
        assert!(!result.valid);
    }

    #[test]
    fn sm_full_tool_loop_cycle() {
        let mut sm = ConversationStateMachine::new();
        // User asks -> query -> tool call -> execute -> query -> text response -> idle
        sm.on_event(DialogueEvent::UserMessage);
        assert_eq!(sm.state(), ConversationState::Querying);
        sm.on_event(DialogueEvent::ToolCallResponse);
        assert_eq!(sm.state(), ConversationState::ToolExecution);
        sm.on_event(DialogueEvent::ToolExecutionDone);
        assert_eq!(sm.state(), ConversationState::Querying);
        sm.on_event(DialogueEvent::TextResponse);
        assert_eq!(sm.state(), ConversationState::Idle);
        assert_eq!(sm.transition_count(), 4);
    }

    // ── PlannedAction ──────────────────────────────────────────────

    #[test]
    fn planned_action_display() {
        assert_eq!(PlannedAction::ContinueToolLoop.to_string(), "continue_tool_loop");
        assert_eq!(PlannedAction::SendToLlm.to_string(), "send_to_llm");
        assert_eq!(PlannedAction::Summarize.to_string(), "summarize");
    }

    // ── plan_next_turn ─────────────────────────────────────────────

    #[test]
    fn plan_finish_on_stop_signal() {
        let ctx = TurnContext {
            has_stop_signal: true,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert!(matches!(action, PlannedAction::Finish { .. }));
    }

    #[test]
    fn plan_finish_on_max_turns() {
        let ctx = TurnContext {
            turn: 51,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert!(matches!(action, PlannedAction::Finish { .. }));
    }

    #[test]
    fn plan_summarize_on_high_context() {
        let ctx = TurnContext {
            context_usage: 0.85,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert_eq!(action, PlannedAction::Summarize);
    }

    #[test]
    fn plan_confirm_after_many_tools() {
        let ctx = TurnContext {
            tool_calls_in_turn: 10,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert!(matches!(action, PlannedAction::RequestUserInput { .. }));
    }

    #[test]
    fn plan_confirm_on_errors() {
        let ctx = TurnContext {
            errors_in_turn: 3,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert!(matches!(action, PlannedAction::RequestUserInput { .. }));
    }

    #[test]
    fn plan_confirm_every_n_turns() {
        let policy = DialoguePolicy {
            confirm_every_n_turns: 5,
            ..Default::default()
        };
        let ctx = TurnContext {
            turn: 5,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &policy);
        assert!(matches!(action, PlannedAction::RequestUserInput { .. }));
    }

    #[test]
    fn plan_no_confirm_when_disabled() {
        let policy = DialoguePolicy {
            confirm_after_tools: 0,
            confirm_every_n_turns: 0,
            ..Default::default()
        };
        let ctx = TurnContext {
            tool_calls_in_turn: 100,
            turn: 10,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &policy);
        // Should not request confirmation
        assert!(matches!(action, PlannedAction::SendToLlm));
    }

    #[test]
    fn plan_send_to_llm_after_tool_calls() {
        let ctx = TurnContext {
            tool_calls_in_turn: 3,
            ..Default::default()
        };
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert_eq!(action, PlannedAction::SendToLlm);
    }

    #[test]
    fn plan_continue_tool_loop_default() {
        let ctx = TurnContext::default();
        let action = plan_next_turn(&ctx, &DialoguePolicy::default());
        assert_eq!(action, PlannedAction::ContinueToolLoop);
    }

    // ── DialoguePolicy ─────────────────────────────────────────────

    #[test]
    fn policy_defaults() {
        let p = DialoguePolicy::default();
        assert_eq!(p.max_turns, 50);
        assert!((p.summarize_threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(p.confirm_after_tools, 10);
        assert!(!p.autonomous);
    }

    #[test]
    fn policy_strict() {
        let p = DialoguePolicy::strict();
        assert_eq!(p.max_turns, 20);
        assert_eq!(p.confirm_after_tools, 5);
        assert_eq!(p.confirm_every_n_turns, 5);
        assert!(!p.autonomous);
    }

    #[test]
    fn policy_autonomous() {
        let p = DialoguePolicy::autonomous();
        assert_eq!(p.max_turns, 200);
        assert_eq!(p.confirm_after_tools, 0);
        assert!(p.autonomous);
        assert!(p.allows_autonomous());
    }

    #[test]
    fn policy_turn_limit() {
        let p = DialoguePolicy::default();
        assert!(!p.is_turn_limit_reached(50));
        assert!(p.is_turn_limit_reached(51));
    }

    #[test]
    fn policy_needs_summarization() {
        let p = DialoguePolicy::default();
        assert!(!p.needs_summarization(0.5));
        assert!(p.needs_summarization(0.8));
        assert!(p.needs_summarization(0.95));
    }

    #[test]
    fn policy_no_auto_summarize() {
        let p = DialoguePolicy {
            auto_summarize: false,
            ..Default::default()
        };
        assert!(!p.needs_summarization(0.95));
    }

    #[test]
    fn policy_summary_format() {
        let p = DialoguePolicy::default();
        let summary = p.to_summary();
        assert!(summary.contains("Max turns: 50"));
        assert!(summary.contains("Autonomous: false"));
    }

    // ── TurnContext defaults ───────────────────────────────────────

    #[test]
    fn turn_context_defaults() {
        let ctx = TurnContext::default();
        assert_eq!(ctx.turn, 1);
        assert_eq!(ctx.tool_calls_in_turn, 0);
        assert!(!ctx.last_needed_confirmation);
        assert!((ctx.context_usage - 0.0).abs() < f64::EPSILON);
        assert!(!ctx.has_stop_signal);
        assert_eq!(ctx.errors_in_turn, 0);
    }

    // ── Integration: state machine + planner ───────────────────────

    #[test]
    fn full_dialogue_flow() {
        let mut sm = ConversationStateMachine::new();
        let policy = DialoguePolicy::default();

        // Turn 1: user message -> query -> tool call -> execute -> query -> text
        sm.on_event(DialogueEvent::UserMessage);
        let ctx = TurnContext { turn: 1, tool_calls_in_turn: 1, ..Default::default() };
        let action = plan_next_turn(&ctx, &policy);
        assert_eq!(action, PlannedAction::SendToLlm);

        sm.on_event(DialogueEvent::ToolCallResponse);
        sm.on_event(DialogueEvent::ToolExecutionDone);
        sm.on_event(DialogueEvent::TextResponse);
        assert_eq!(sm.state(), ConversationState::Idle);
    }

    #[test]
    fn dialogue_with_summarization() {
        let mut sm = ConversationStateMachine::new();
        let policy = DialoguePolicy::default();

        let ctx = TurnContext { context_usage: 0.9, ..Default::default() };
        let action = plan_next_turn(&ctx, &policy);
        assert_eq!(action, PlannedAction::Summarize);

        sm.on_event(DialogueEvent::SummarizeTriggered);
        assert_eq!(sm.state(), ConversationState::Summarizing);
        sm.on_event(DialogueEvent::SummarizeDone);
        assert_eq!(sm.state(), ConversationState::Idle);
    }
}
