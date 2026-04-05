//! Tool execution pipelines and chains.
//!
//! Provides composable tool pipelines where the output of one step feeds
//! into the next, with conditional execution and result aggregation.

use crab_core::tool::{ToolContext, ToolOutput};
use crab_tools::executor::ToolExecutor;
use serde_json::Value;
use std::time::{Duration, Instant};

/// A single step in a tool pipeline.
#[derive(Debug, Clone)]
pub struct PipelineStep {
    /// Tool name to execute.
    pub tool_name: String,
    /// Static input fields (merged with piped output).
    pub base_input: Value,
    /// Key in the next step's input where this step's output text is injected.
    /// If `None`, the output is not piped forward (terminal step).
    pub output_key: Option<String>,
    /// Condition that must be met for this step to execute.
    pub condition: StepCondition,
}

/// Condition for whether a pipeline step should execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepCondition {
    /// Always execute.
    Always,
    /// Only execute if the previous step succeeded (not an error).
    PreviousSucceeded,
    /// Only execute if the previous step's output contains the given text.
    OutputContains(String),
    /// Only execute if the previous step's output does NOT contain the given text.
    OutputNotContains(String),
}

impl StepCondition {
    /// Evaluate the condition against the previous step's result.
    fn evaluate(&self, prev: Option<&StepResult>) -> bool {
        match self {
            Self::Always => true,
            Self::PreviousSucceeded => prev.is_none_or(|r| r.success),
            Self::OutputContains(text) => {
                prev.is_some_and(|r| r.output_text.contains(text.as_str()))
            }
            Self::OutputNotContains(text) => {
                prev.is_none_or(|r| !r.output_text.contains(text.as_str()))
            }
        }
    }
}

/// Result of a single pipeline step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// The tool that was executed.
    pub tool_name: String,
    /// Whether the step succeeded.
    pub success: bool,
    /// The text output from the tool.
    pub output_text: String,
    /// The full tool output.
    pub output: ToolOutput,
    /// How long the step took.
    pub duration: Duration,
    /// Whether this step was skipped due to a condition.
    pub skipped: bool,
}

/// Aggregated result of a full pipeline execution.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Results of each step, in execution order.
    pub steps: Vec<StepResult>,
    /// Total pipeline duration.
    pub total_duration: Duration,
    /// Whether the entire pipeline succeeded (all non-skipped steps succeeded).
    pub success: bool,
}

impl PipelineResult {
    /// Get the output text of the last executed (non-skipped) step.
    #[must_use]
    pub fn final_output(&self) -> Option<&str> {
        self.steps
            .iter()
            .rev()
            .find(|s| !s.skipped)
            .map(|s| s.output_text.as_str())
    }

    /// Number of steps that were actually executed (not skipped).
    #[must_use]
    pub fn executed_count(&self) -> usize {
        self.steps.iter().filter(|s| !s.skipped).count()
    }

    /// Number of steps that were skipped.
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.steps.iter().filter(|s| s.skipped).count()
    }

    /// Number of steps that failed.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| !s.skipped && !s.success)
            .count()
    }
}

/// A pipeline that executes tools in sequence, piping outputs forward.
pub struct ToolPipeline {
    steps: Vec<PipelineStep>,
    /// If true, stop the pipeline on the first error.
    stop_on_error: bool,
}

impl ToolPipeline {
    /// Create a new empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            stop_on_error: true,
        }
    }

    /// Set whether to stop on the first error (default: true).
    #[must_use]
    pub fn stop_on_error(mut self, stop: bool) -> Self {
        self.stop_on_error = stop;
        self
    }

    /// Add a step to the pipeline.
    #[must_use]
    pub fn step(mut self, step: PipelineStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Add a simple step: tool name + input, always execute, pipe output to given key.
    #[must_use]
    pub fn then(
        self,
        tool_name: impl Into<String>,
        input: Value,
        output_key: Option<String>,
    ) -> Self {
        self.step(PipelineStep {
            tool_name: tool_name.into(),
            base_input: input,
            output_key,
            condition: StepCondition::Always,
        })
    }

    /// Add a conditional step that only runs if the previous step succeeded.
    #[must_use]
    pub fn then_if_ok(
        self,
        tool_name: impl Into<String>,
        input: Value,
        output_key: Option<String>,
    ) -> Self {
        self.step(PipelineStep {
            tool_name: tool_name.into(),
            base_input: input,
            output_key,
            condition: StepCondition::PreviousSucceeded,
        })
    }

    /// Number of steps in the pipeline.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the pipeline has no steps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Execute the pipeline.
    pub async fn execute(&self, executor: &ToolExecutor, ctx: &ToolContext) -> PipelineResult {
        let pipeline_start = Instant::now();
        let mut results = Vec::with_capacity(self.steps.len());
        let mut prev_result: Option<StepResult> = None;

        for step in &self.steps {
            // Check condition
            if !step.condition.evaluate(prev_result.as_ref()) {
                results.push(StepResult {
                    tool_name: step.tool_name.clone(),
                    success: true,
                    output_text: String::new(),
                    output: ToolOutput::success(""),
                    duration: Duration::ZERO,
                    skipped: true,
                });
                continue;
            }

            // Build input: merge base_input with piped output from previous step
            let input = build_step_input(step, prev_result.as_ref());

            // Execute
            let step_start = Instant::now();
            let result = executor.execute(&step.tool_name, input, ctx).await;
            let duration = step_start.elapsed();

            let step_result = match result {
                Ok(output) => {
                    let text = output.text();
                    let success = !output.is_error;
                    StepResult {
                        tool_name: step.tool_name.clone(),
                        success,
                        output_text: text,
                        output,
                        duration,
                        skipped: false,
                    }
                }
                Err(e) => StepResult {
                    tool_name: step.tool_name.clone(),
                    success: false,
                    output_text: e.to_string(),
                    output: ToolOutput::error(e.to_string()),
                    duration,
                    skipped: false,
                },
            };

            let should_stop = self.stop_on_error && !step_result.success;
            prev_result = Some(step_result.clone());
            results.push(step_result);

            if should_stop {
                break;
            }
        }

        let success = results.iter().all(|r| r.skipped || r.success);
        PipelineResult {
            steps: results,
            total_duration: pipeline_start.elapsed(),
            success,
        }
    }
}

impl Default for ToolPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the input for a step by merging `base_input` with piped output.
fn build_step_input(step: &PipelineStep, prev: Option<&StepResult>) -> Value {
    let mut input = step.base_input.clone();

    // If there's a previous result and the previous step had an output_key configured,
    // we look at the *previous step's* output_key to know which field to inject into.
    // But actually, the output_key on a step says where THIS step's output goes.
    // For piping, we need to inject the prev output into this step's input.
    // Convention: if the prev step's output_key is set, inject into that key of this input.
    if let Some(prev_result) = prev {
        // Find what key the previous step wants to pipe its output into
        // We scan backwards — not ideal. Instead, use a simpler approach:
        // If this step's base_input has a key "__pipe__", replace it with prev output.
        if let Value::Object(map) = &mut input {
            // Replace any value that is the string "__pipe__" with the previous output
            for value in map.values_mut() {
                if value.as_str() == Some("__pipe__") {
                    *value = Value::String(prev_result.output_text.clone());
                }
            }
        }
    }

    input
}

/// Pre-defined tool chain templates.
pub struct ToolChain;

impl ToolChain {
    /// Create a grep -> read chain: search for a pattern, then read matching files.
    #[must_use]
    pub fn grep_then_read(pattern: &str, path: &str) -> ToolPipeline {
        ToolPipeline::new()
            .then(
                "grep",
                serde_json::json!({
                    "pattern": pattern,
                    "path": path,
                    "output_mode": "files_with_matches"
                }),
                Some("files".into()),
            )
            .then_if_ok(
                "read",
                serde_json::json!({
                    "file_path": "__pipe__"
                }),
                None,
            )
    }

    /// Create a glob -> read chain: find files by pattern, then read them.
    #[must_use]
    pub fn glob_then_read(pattern: &str) -> ToolPipeline {
        ToolPipeline::new()
            .then(
                "glob",
                serde_json::json!({
                    "pattern": pattern
                }),
                Some("files".into()),
            )
            .then_if_ok(
                "read",
                serde_json::json!({
                    "file_path": "__pipe__"
                }),
                None,
            )
    }

    /// Create a read -> edit chain: read a file, then edit it.
    #[must_use]
    pub fn read_then_edit(file_path: &str, old_string: &str, new_string: &str) -> ToolPipeline {
        ToolPipeline::new()
            .then(
                "read",
                serde_json::json!({
                    "file_path": file_path
                }),
                None,
            )
            .then_if_ok(
                "edit",
                serde_json::json!({
                    "file_path": file_path,
                    "old_string": old_string,
                    "new_string": new_string
                }),
                None,
            )
    }

    /// Create a custom pipeline from a list of (`tool_name`, input) pairs.
    /// Each step is conditional on the previous succeeding.
    #[must_use]
    pub fn from_steps(steps: Vec<(String, Value)>) -> ToolPipeline {
        let mut pipeline = ToolPipeline::new();
        for (i, (name, input)) in steps.into_iter().enumerate() {
            let condition = if i == 0 {
                StepCondition::Always
            } else {
                StepCondition::PreviousSucceeded
            };
            pipeline = pipeline.step(PipelineStep {
                tool_name: name,
                base_input: input,
                output_key: None,
                condition,
            });
        }
        pipeline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── StepCondition ───────────────────────────────────────────────

    #[test]
    fn condition_always_evaluates_true() {
        assert!(StepCondition::Always.evaluate(None));
        assert!(StepCondition::Always.evaluate(Some(&make_result(true, "ok"))));
        assert!(StepCondition::Always.evaluate(Some(&make_result(false, "err"))));
    }

    #[test]
    fn condition_previous_succeeded_no_prev() {
        assert!(StepCondition::PreviousSucceeded.evaluate(None));
    }

    #[test]
    fn condition_previous_succeeded_true() {
        assert!(StepCondition::PreviousSucceeded.evaluate(Some(&make_result(true, "ok"))));
    }

    #[test]
    fn condition_previous_succeeded_false() {
        assert!(!StepCondition::PreviousSucceeded.evaluate(Some(&make_result(false, "err"))));
    }

    #[test]
    fn condition_output_contains_match() {
        let cond = StepCondition::OutputContains("found".into());
        assert!(cond.evaluate(Some(&make_result(true, "file found here"))));
    }

    #[test]
    fn condition_output_contains_no_match() {
        let cond = StepCondition::OutputContains("found".into());
        assert!(!cond.evaluate(Some(&make_result(true, "nothing"))));
    }

    #[test]
    fn condition_output_contains_no_prev() {
        let cond = StepCondition::OutputContains("found".into());
        assert!(!cond.evaluate(None));
    }

    #[test]
    fn condition_output_not_contains_match() {
        let cond = StepCondition::OutputNotContains("error".into());
        assert!(cond.evaluate(Some(&make_result(true, "all good"))));
    }

    #[test]
    fn condition_output_not_contains_no_match() {
        let cond = StepCondition::OutputNotContains("error".into());
        assert!(!cond.evaluate(Some(&make_result(true, "error occurred"))));
    }

    #[test]
    fn condition_output_not_contains_no_prev() {
        let cond = StepCondition::OutputNotContains("error".into());
        assert!(cond.evaluate(None));
    }

    // ── PipelineStep construction ───────────────────────────────────

    #[test]
    fn pipeline_step_fields() {
        let step = PipelineStep {
            tool_name: "read".into(),
            base_input: serde_json::json!({"file_path": "/tmp/x"}),
            output_key: Some("content".into()),
            condition: StepCondition::Always,
        };
        assert_eq!(step.tool_name, "read");
        assert_eq!(step.output_key.as_deref(), Some("content"));
    }

    // ── ToolPipeline builder ────────────────────────────────────────

    #[test]
    fn pipeline_new_is_empty() {
        let p = ToolPipeline::new();
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn pipeline_default_is_empty() {
        let p = ToolPipeline::default();
        assert!(p.is_empty());
    }

    #[test]
    fn pipeline_then_adds_steps() {
        let p = ToolPipeline::new()
            .then("grep", serde_json::json!({}), None)
            .then("read", serde_json::json!({}), None);
        assert_eq!(p.len(), 2);
        assert!(!p.is_empty());
    }

    #[test]
    fn pipeline_then_if_ok_sets_condition() {
        let p = ToolPipeline::new()
            .then("grep", serde_json::json!({}), None)
            .then_if_ok("read", serde_json::json!({}), None);
        assert_eq!(p.steps[0].condition, StepCondition::Always);
        assert_eq!(p.steps[1].condition, StepCondition::PreviousSucceeded);
    }

    #[test]
    fn pipeline_stop_on_error_builder() {
        let p = ToolPipeline::new().stop_on_error(false);
        assert!(!p.stop_on_error);
    }

    // ── build_step_input ────────────────────────────────────────────

    #[test]
    fn build_input_no_prev() {
        let step = PipelineStep {
            tool_name: "read".into(),
            base_input: serde_json::json!({"file_path": "/tmp/x"}),
            output_key: None,
            condition: StepCondition::Always,
        };
        let input = build_step_input(&step, None);
        assert_eq!(input["file_path"], "/tmp/x");
    }

    #[test]
    fn build_input_with_pipe_replacement() {
        let step = PipelineStep {
            tool_name: "read".into(),
            base_input: serde_json::json!({"file_path": "__pipe__"}),
            output_key: None,
            condition: StepCondition::Always,
        };
        let prev = make_result(true, "/tmp/found.rs");
        let input = build_step_input(&step, Some(&prev));
        assert_eq!(input["file_path"], "/tmp/found.rs");
    }

    #[test]
    fn build_input_no_pipe_marker_unchanged() {
        let step = PipelineStep {
            tool_name: "edit".into(),
            base_input: serde_json::json!({"file_path": "/tmp/x", "old": "a", "new": "b"}),
            output_key: None,
            condition: StepCondition::Always,
        };
        let prev = make_result(true, "some output");
        let input = build_step_input(&step, Some(&prev));
        assert_eq!(input["file_path"], "/tmp/x");
        assert_eq!(input["old"], "a");
    }

    // ── PipelineResult ──────────────────────────────────────────────

    #[test]
    fn pipeline_result_final_output() {
        let result = PipelineResult {
            steps: vec![
                make_result(true, "step1"),
                make_step_result(true, "step2", false),
            ],
            total_duration: Duration::from_millis(100),
            success: true,
        };
        assert_eq!(result.final_output(), Some("step2"));
    }

    #[test]
    fn pipeline_result_final_output_skips_skipped() {
        let result = PipelineResult {
            steps: vec![
                make_step_result(true, "step1", false),
                make_step_result(true, "step2", true), // skipped
            ],
            total_duration: Duration::from_millis(100),
            success: true,
        };
        assert_eq!(result.final_output(), Some("step1"));
    }

    #[test]
    fn pipeline_result_counts() {
        let result = PipelineResult {
            steps: vec![
                make_step_result(true, "ok", false),
                make_step_result(false, "err", false),
                make_step_result(true, "", true), // skipped
            ],
            total_duration: Duration::from_millis(50),
            success: false,
        };
        assert_eq!(result.executed_count(), 2);
        assert_eq!(result.skipped_count(), 1);
        assert_eq!(result.failed_count(), 1);
    }

    #[test]
    fn pipeline_result_empty() {
        let result = PipelineResult {
            steps: vec![],
            total_duration: Duration::ZERO,
            success: true,
        };
        assert!(result.final_output().is_none());
        assert_eq!(result.executed_count(), 0);
        assert_eq!(result.skipped_count(), 0);
        assert_eq!(result.failed_count(), 0);
    }

    // ── ToolChain factory methods ───────────────────────────────────

    #[test]
    fn chain_grep_then_read() {
        let p = ToolChain::grep_then_read("TODO", "src/");
        assert_eq!(p.len(), 2);
        assert_eq!(p.steps[0].tool_name, "grep");
        assert_eq!(p.steps[1].tool_name, "read");
        assert_eq!(p.steps[1].condition, StepCondition::PreviousSucceeded);
    }

    #[test]
    fn chain_glob_then_read() {
        let p = ToolChain::glob_then_read("**/*.rs");
        assert_eq!(p.len(), 2);
        assert_eq!(p.steps[0].tool_name, "glob");
        assert_eq!(p.steps[1].tool_name, "read");
    }

    #[test]
    fn chain_read_then_edit() {
        let p = ToolChain::read_then_edit("/tmp/x.rs", "old", "new");
        assert_eq!(p.len(), 2);
        assert_eq!(p.steps[0].tool_name, "read");
        assert_eq!(p.steps[1].tool_name, "edit");
        assert_eq!(p.steps[1].base_input["old_string"], "old");
    }

    #[test]
    fn chain_from_steps() {
        let p = ToolChain::from_steps(vec![
            ("grep".into(), serde_json::json!({"pattern": "fn main"})),
            ("read".into(), serde_json::json!({"file_path": "__pipe__"})),
            ("edit".into(), serde_json::json!({"file_path": "/x"})),
        ]);
        assert_eq!(p.len(), 3);
        assert_eq!(p.steps[0].condition, StepCondition::Always);
        assert_eq!(p.steps[1].condition, StepCondition::PreviousSucceeded);
        assert_eq!(p.steps[2].condition, StepCondition::PreviousSucceeded);
    }

    #[test]
    fn chain_from_steps_empty() {
        let p = ToolChain::from_steps(vec![]);
        assert!(p.is_empty());
    }

    // ── Integration: pipeline with real executor (empty registry) ───

    #[tokio::test]
    async fn pipeline_execute_unknown_tool_fails() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let executor = ToolExecutor::new(std::sync::Arc::new(registry));
        let ctx = make_ctx();

        let pipeline = ToolPipeline::new().then("nonexistent_tool", serde_json::json!({}), None);

        let result = pipeline.execute(&executor, &ctx).await;
        assert!(!result.success);
        assert_eq!(result.steps.len(), 1);
        assert!(!result.steps[0].success);
    }

    #[tokio::test]
    async fn pipeline_stop_on_error_halts_chain() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let executor = ToolExecutor::new(std::sync::Arc::new(registry));
        let ctx = make_ctx();

        let pipeline = ToolPipeline::new()
            .stop_on_error(true)
            .then("bad_tool", serde_json::json!({}), None)
            .then("also_bad", serde_json::json!({}), None);

        let result = pipeline.execute(&executor, &ctx).await;
        assert!(!result.success);
        // Second step should not have executed
        assert_eq!(result.steps.len(), 1);
    }

    #[tokio::test]
    async fn pipeline_continue_on_error() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let executor = ToolExecutor::new(std::sync::Arc::new(registry));
        let ctx = make_ctx();

        let pipeline = ToolPipeline::new()
            .stop_on_error(false)
            .then("bad1", serde_json::json!({}), None)
            .then("bad2", serde_json::json!({}), None);

        let result = pipeline.execute(&executor, &ctx).await;
        assert!(!result.success);
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.failed_count(), 2);
    }

    #[tokio::test]
    async fn pipeline_condition_skips_step() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let executor = ToolExecutor::new(std::sync::Arc::new(registry));
        let ctx = make_ctx();

        let pipeline = ToolPipeline::new()
            .stop_on_error(false)
            .then("bad_tool", serde_json::json!({}), None)
            .then_if_ok("should_skip", serde_json::json!({}), None);

        let result = pipeline.execute(&executor, &ctx).await;
        assert_eq!(result.steps.len(), 2);
        assert!(!result.steps[0].success);
        assert!(result.steps[1].skipped);
        assert_eq!(result.skipped_count(), 1);
    }

    #[tokio::test]
    async fn pipeline_empty_succeeds() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let executor = ToolExecutor::new(std::sync::Arc::new(registry));
        let ctx = make_ctx();

        let pipeline = ToolPipeline::new();
        let result = pipeline.execute(&executor, &ctx).await;
        assert!(result.success);
        assert!(result.steps.is_empty());
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn make_result(success: bool, text: &str) -> StepResult {
        make_step_result(success, text, false)
    }

    fn make_step_result(success: bool, text: &str, skipped: bool) -> StepResult {
        StepResult {
            tool_name: "test".into(),
            success,
            output_text: text.into(),
            output: if success {
                ToolOutput::success(text)
            } else {
                ToolOutput::error(text)
            },
            duration: Duration::from_millis(10),
            skipped,
        }
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::path::PathBuf::from("."),
            permission_mode: crab_core::permission::PermissionMode::Dangerously,
            session_id: "test".into(),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            permission_policy: crab_core::permission::PermissionPolicy {
                mode: crab_core::permission::PermissionMode::Dangerously,
                allowed_tools: Vec::new(),
                denied_tools: Vec::new(),
            },
        }
    }
}
