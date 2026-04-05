//! Adaptive system prompt and dynamic tool selection.
//!
//! Analyzes conversation context to dynamically adjust the system prompt
//! and enable/disable tool subsets based on the detected task type.

use std::collections::HashMap;
use std::fmt::Write;

/// Detected context type for the current conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextType {
    /// Debugging an issue (error messages, stack traces, "fix", "bug").
    Debugging,
    /// Code writing/generation ("create", "implement", "add feature").
    CodeGeneration,
    /// Code review / explanation ("explain", "review", "what does").
    CodeReview,
    /// Refactoring ("refactor", "rename", "extract", "move").
    Refactoring,
    /// File/project navigation ("find", "search", "where is").
    Navigation,
    /// Testing ("test", "assert", "coverage").
    Testing,
    /// Documentation ("document", "docstring", "readme").
    Documentation,
    /// General conversation (no specific pattern detected).
    General,
}

impl std::fmt::Display for ContextType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debugging => write!(f, "debugging"),
            Self::CodeGeneration => write!(f, "code_generation"),
            Self::CodeReview => write!(f, "code_review"),
            Self::Refactoring => write!(f, "refactoring"),
            Self::Navigation => write!(f, "navigation"),
            Self::Testing => write!(f, "testing"),
            Self::Documentation => write!(f, "documentation"),
            Self::General => write!(f, "general"),
        }
    }
}

/// Keywords that signal each context type, with their weights.
struct ContextSignal {
    context: ContextType,
    keywords: &'static [&'static str],
}

const CONTEXT_SIGNALS: &[ContextSignal] = &[
    ContextSignal {
        context: ContextType::Debugging,
        keywords: &[
            "fix",
            "bug",
            "error",
            "crash",
            "panic",
            "fail",
            "broken",
            "debug",
            "stack trace",
            "traceback",
            "exception",
            "issue",
            "wrong",
        ],
    },
    ContextSignal {
        context: ContextType::CodeGeneration,
        keywords: &[
            "create",
            "implement",
            "add",
            "build",
            "generate",
            "write",
            "new feature",
            "scaffold",
            "boilerplate",
        ],
    },
    ContextSignal {
        context: ContextType::CodeReview,
        keywords: &[
            "explain",
            "review",
            "what does",
            "how does",
            "understand",
            "walk through",
            "describe",
            "analyze",
        ],
    },
    ContextSignal {
        context: ContextType::Refactoring,
        keywords: &[
            "refactor",
            "rename",
            "extract",
            "move",
            "reorganize",
            "clean up",
            "simplify",
            "restructure",
            "dedup",
        ],
    },
    ContextSignal {
        context: ContextType::Navigation,
        keywords: &[
            "find",
            "search",
            "where",
            "locate",
            "grep",
            "glob",
            "list files",
            "show me",
            "look for",
        ],
    },
    ContextSignal {
        context: ContextType::Testing,
        keywords: &[
            "test",
            "assert",
            "coverage",
            "spec",
            "unit test",
            "integration test",
            "mock",
            "fixture",
            "tdd",
        ],
    },
    ContextSignal {
        context: ContextType::Documentation,
        keywords: &[
            "document",
            "docstring",
            "readme",
            "comment",
            "doc",
            "markdown",
            "changelog",
            "api doc",
        ],
    },
];

/// Detect the most likely context type from user input text.
#[must_use]
pub fn detect_context(input: &str) -> ContextType {
    let lower = input.to_lowercase();
    let mut scores: HashMap<ContextType, usize> = HashMap::new();

    for signal in CONTEXT_SIGNALS {
        let count: usize = signal
            .keywords
            .iter()
            .filter(|kw| lower.contains(*kw))
            .count();
        if count > 0 {
            *scores.entry(signal.context).or_default() += count;
        }
    }

    scores
        .into_iter()
        .max_by_key(|(_, score)| *score)
        .map_or(ContextType::General, |(ctx, _)| ctx)
}

// ── Prompt templates ────────────────────────────────────────────────

/// A named prompt fragment that can be composed into a system prompt.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    /// Unique name for this template.
    pub name: String,
    /// The template content.
    pub content: String,
    /// Priority for ordering (lower = earlier in prompt).
    pub priority: u32,
}

/// Registry of prompt templates that can be composed based on context.
#[derive(Debug, Clone, Default)]
pub struct PromptTemplateRegistry {
    templates: Vec<PromptTemplate>,
    /// Maps context type to template names that should be activated.
    context_bindings: HashMap<ContextType, Vec<String>>,
}

impl PromptTemplateRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry pre-loaded with default templates.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();

        reg.add_template(PromptTemplate {
            name: "debugging".into(),
            content: concat!(
                "You are in debugging mode. Focus on:\n",
                "- Reading error messages carefully\n",
                "- Checking recent changes with git diff/log\n",
                "- Using grep to locate error sources\n",
                "- Running tests to reproduce issues\n",
                "- Making minimal, targeted fixes\n",
            )
            .into(),
            priority: 10,
        });

        reg.add_template(PromptTemplate {
            name: "code_generation".into(),
            content: concat!(
                "You are in code generation mode. Focus on:\n",
                "- Understanding existing patterns in the codebase\n",
                "- Following project conventions and style\n",
                "- Writing tests alongside new code\n",
                "- Using edit/write tools for implementation\n",
            )
            .into(),
            priority: 10,
        });

        reg.add_template(PromptTemplate {
            name: "code_review".into(),
            content: concat!(
                "You are in code review mode. Focus on:\n",
                "- Reading code thoroughly before commenting\n",
                "- Explaining logic step by step\n",
                "- Identifying potential issues or improvements\n",
                "- Being constructive and specific\n",
            )
            .into(),
            priority: 10,
        });

        reg.add_template(PromptTemplate {
            name: "refactoring".into(),
            content: concat!(
                "You are in refactoring mode. Focus on:\n",
                "- Understanding the full scope of changes needed\n",
                "- Using grep to find all references before renaming\n",
                "- Making changes incrementally and testing between steps\n",
                "- Preserving behavior while improving structure\n",
            )
            .into(),
            priority: 10,
        });

        reg.add_template(PromptTemplate {
            name: "navigation".into(),
            content: concat!(
                "You are in navigation mode. Focus on:\n",
                "- Using glob and grep to locate files and patterns\n",
                "- Reading file headers and structure, not full contents\n",
                "- Summarizing findings concisely\n",
            )
            .into(),
            priority: 10,
        });

        reg.add_template(PromptTemplate {
            name: "testing".into(),
            content: concat!(
                "You are in testing mode. Focus on:\n",
                "- Writing comprehensive test cases\n",
                "- Covering edge cases and error paths\n",
                "- Running tests after writing them\n",
                "- Following existing test patterns in the project\n",
            )
            .into(),
            priority: 10,
        });

        reg.add_template(PromptTemplate {
            name: "documentation".into(),
            content: concat!(
                "You are in documentation mode. Focus on:\n",
                "- Reading the code before documenting\n",
                "- Writing clear, concise documentation\n",
                "- Including examples where helpful\n",
                "- Matching existing documentation style\n",
            )
            .into(),
            priority: 10,
        });

        // Bind templates to context types
        reg.bind(ContextType::Debugging, "debugging");
        reg.bind(ContextType::CodeGeneration, "code_generation");
        reg.bind(ContextType::CodeReview, "code_review");
        reg.bind(ContextType::Refactoring, "refactoring");
        reg.bind(ContextType::Navigation, "navigation");
        reg.bind(ContextType::Testing, "testing");
        reg.bind(ContextType::Documentation, "documentation");

        reg
    }

    /// Add a template to the registry.
    pub fn add_template(&mut self, template: PromptTemplate) {
        self.templates.push(template);
    }

    /// Bind a template name to a context type.
    pub fn bind(&mut self, context: ContextType, template_name: &str) {
        self.context_bindings
            .entry(context)
            .or_default()
            .push(template_name.to_string());
    }

    /// Get templates activated by a given context type, sorted by priority.
    #[must_use]
    pub fn templates_for_context(&self, context: ContextType) -> Vec<&PromptTemplate> {
        let Some(names) = self.context_bindings.get(&context) else {
            return Vec::new();
        };

        let mut templates: Vec<&PromptTemplate> = self
            .templates
            .iter()
            .filter(|t| names.contains(&t.name))
            .collect();
        templates.sort_by_key(|t| t.priority);
        templates
    }

    /// Compose activated templates into a single prompt section.
    #[must_use]
    pub fn compose_for_context(&self, context: ContextType) -> String {
        let templates = self.templates_for_context(context);
        if templates.is_empty() {
            return String::new();
        }

        let mut result = String::new();
        let _ = writeln!(result, "# Context-Adaptive Instructions\n");
        let _ = writeln!(result, "Detected task type: {context}\n");
        for t in templates {
            let _ = writeln!(result, "{}\n", t.content);
        }
        result
    }

    /// Number of registered templates.
    #[must_use]
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }
}

// ── Dynamic tool selection ──────────────────────────────────────────

/// Mapping from context type to recommended tool subsets.
#[derive(Debug, Clone, Default)]
pub struct ToolSelector {
    /// Maps context type to sets of tool names.
    context_tools: HashMap<ContextType, Vec<String>>,
}

impl ToolSelector {
    /// Create a new empty selector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a selector with default tool-context mappings.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut sel = Self::new();

        sel.register(
            ContextType::Debugging,
            &["read", "grep", "bash", "glob", "edit"],
        );
        sel.register(
            ContextType::CodeGeneration,
            &["read", "write", "edit", "bash", "glob", "grep"],
        );
        sel.register(ContextType::CodeReview, &["read", "grep", "glob"]);
        sel.register(
            ContextType::Refactoring,
            &["read", "edit", "grep", "glob", "bash"],
        );
        sel.register(ContextType::Navigation, &["glob", "grep", "read"]);
        sel.register(
            ContextType::Testing,
            &["read", "write", "edit", "bash", "grep"],
        );
        sel.register(
            ContextType::Documentation,
            &["read", "write", "edit", "glob"],
        );
        // General: all tools
        sel.register(
            ContextType::General,
            &["read", "write", "edit", "bash", "glob", "grep"],
        );

        sel
    }

    /// Register tool names for a context type.
    pub fn register(&mut self, context: ContextType, tools: &[&str]) {
        self.context_tools
            .insert(context, tools.iter().map(|s| (*s).to_string()).collect());
    }

    /// Get recommended tools for a context type.
    #[must_use]
    pub fn tools_for_context(&self, context: ContextType) -> &[String] {
        self.context_tools.get(&context).map_or(&[], Vec::as_slice)
    }

    /// Check if a tool is recommended for the given context.
    #[must_use]
    pub fn is_recommended(&self, context: ContextType, tool_name: &str) -> bool {
        self.tools_for_context(context)
            .iter()
            .any(|t| t == tool_name)
    }
}

// ── Tool usage tracker / recommender ────────────────────────────────

/// Tracks tool usage history and recommends tools based on patterns.
#[derive(Debug, Clone, Default)]
pub struct ToolRecommender {
    /// Usage counts: `tool_name` -> `total_uses`.
    usage_counts: HashMap<String, u64>,
    /// Co-occurrence: (`tool_a`, `tool_b`) -> times used together in same turn.
    co_occurrences: HashMap<(String, String), u64>,
    /// Context-specific usage: (context, tool) -> count.
    context_usage: HashMap<(ContextType, String), u64>,
}

impl ToolRecommender {
    /// Create a new empty recommender.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a tool was used.
    pub fn record_usage(&mut self, tool_name: &str) {
        *self.usage_counts.entry(tool_name.to_string()).or_default() += 1;
    }

    /// Record that a tool was used in a specific context.
    pub fn record_context_usage(&mut self, context: ContextType, tool_name: &str) {
        *self
            .context_usage
            .entry((context, tool_name.to_string()))
            .or_default() += 1;
    }

    /// Record that multiple tools were used together in the same turn.
    pub fn record_co_occurrence(&mut self, tools: &[&str]) {
        for i in 0..tools.len() {
            for j in (i + 1)..tools.len() {
                let key = if tools[i] <= tools[j] {
                    (tools[i].to_string(), tools[j].to_string())
                } else {
                    (tools[j].to_string(), tools[i].to_string())
                };
                *self.co_occurrences.entry(key).or_default() += 1;
            }
        }
    }

    /// Get the top N most-used tools overall.
    #[must_use]
    pub fn top_tools(&self, n: usize) -> Vec<(&str, u64)> {
        let mut entries: Vec<_> = self
            .usage_counts
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }

    /// Get the top N most-used tools for a specific context.
    #[must_use]
    pub fn top_tools_for_context(&self, context: ContextType, n: usize) -> Vec<(&str, u64)> {
        let mut entries: Vec<_> = self
            .context_usage
            .iter()
            .filter(|((ctx, _), _)| *ctx == context)
            .map(|((_, name), count)| (name.as_str(), *count))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }

    /// Recommend tools that are commonly used alongside the given tool.
    #[must_use]
    pub fn related_tools(&self, tool_name: &str, n: usize) -> Vec<(&str, u64)> {
        let mut related: Vec<_> = self
            .co_occurrences
            .iter()
            .filter_map(|((a, b), count)| {
                if a == tool_name {
                    Some((b.as_str(), *count))
                } else if b == tool_name {
                    Some((a.as_str(), *count))
                } else {
                    None
                }
            })
            .collect();
        related.sort_by(|a, b| b.1.cmp(&a.1));
        related.truncate(n);
        related
    }

    /// Total number of tool usages recorded.
    #[must_use]
    pub fn total_usages(&self) -> u64 {
        self.usage_counts.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Context detection ───────────────────────────────────────────

    #[test]
    fn detect_debugging_context() {
        assert_eq!(
            detect_context("fix the bug in main.rs"),
            ContextType::Debugging
        );
        assert_eq!(
            detect_context("there's an error in the output"),
            ContextType::Debugging
        );
        assert_eq!(
            detect_context("the program crashes on startup"),
            ContextType::Debugging
        );
    }

    #[test]
    fn detect_code_generation_context() {
        assert_eq!(
            detect_context("create a new REST API endpoint"),
            ContextType::CodeGeneration
        );
        assert_eq!(
            detect_context("implement the user authentication feature"),
            ContextType::CodeGeneration
        );
    }

    #[test]
    fn detect_code_review_context() {
        assert_eq!(
            detect_context("explain what this function does"),
            ContextType::CodeReview
        );
        assert_eq!(
            detect_context("review the changes in this PR"),
            ContextType::CodeReview
        );
    }

    #[test]
    fn detect_refactoring_context() {
        assert_eq!(
            detect_context("refactor the database module"),
            ContextType::Refactoring
        );
        assert_eq!(
            detect_context("rename this function to something better"),
            ContextType::Refactoring
        );
    }

    #[test]
    fn detect_navigation_context() {
        assert_eq!(
            detect_context("find all files that use the Config struct"),
            ContextType::Navigation
        );
        assert_eq!(
            detect_context("where is the main function defined"),
            ContextType::Navigation
        );
    }

    #[test]
    fn detect_testing_context() {
        assert_eq!(
            detect_context("write unit tests for the parser"),
            ContextType::Testing
        );
        assert_eq!(
            detect_context("add test coverage for edge cases"),
            ContextType::Testing
        );
    }

    #[test]
    fn detect_documentation_context() {
        assert_eq!(
            detect_context("document the public API"),
            ContextType::Documentation
        );
        assert_eq!(
            detect_context("update the readme with installation steps"),
            ContextType::Documentation
        );
    }

    #[test]
    fn detect_general_context() {
        assert_eq!(detect_context("hello"), ContextType::General);
        assert_eq!(detect_context("what time is it"), ContextType::General);
    }

    #[test]
    fn detect_mixed_signals_picks_strongest() {
        // "fix" (debug) + "test" (testing) — both 1 keyword, but "fix" comes first
        // Actually depends on HashMap ordering; both have 1 match
        let ctx = detect_context("fix the test");
        assert!(ctx == ContextType::Debugging || ctx == ContextType::Testing);
    }

    // ── ContextType Display ─────────────────────────────────────────

    #[test]
    fn context_type_display() {
        assert_eq!(ContextType::Debugging.to_string(), "debugging");
        assert_eq!(ContextType::CodeGeneration.to_string(), "code_generation");
        assert_eq!(ContextType::General.to_string(), "general");
    }

    // ── Prompt template registry ────────────────────────────────────

    #[test]
    fn template_registry_new_is_empty() {
        let reg = PromptTemplateRegistry::new();
        assert_eq!(reg.template_count(), 0);
    }

    #[test]
    fn template_registry_with_defaults_has_templates() {
        let reg = PromptTemplateRegistry::with_defaults();
        assert!(reg.template_count() >= 7);
    }

    #[test]
    fn template_registry_add_and_retrieve() {
        let mut reg = PromptTemplateRegistry::new();
        reg.add_template(PromptTemplate {
            name: "custom".into(),
            content: "Custom instructions.".into(),
            priority: 5,
        });
        reg.bind(ContextType::General, "custom");

        let templates = reg.templates_for_context(ContextType::General);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "custom");
    }

    #[test]
    fn template_registry_no_binding_returns_empty() {
        let reg = PromptTemplateRegistry::new();
        assert!(reg.templates_for_context(ContextType::Debugging).is_empty());
    }

    #[test]
    fn compose_for_context_includes_header() {
        let reg = PromptTemplateRegistry::with_defaults();
        let composed = reg.compose_for_context(ContextType::Debugging);
        assert!(composed.contains("Context-Adaptive Instructions"));
        assert!(composed.contains("debugging"));
        assert!(composed.contains("error messages"));
    }

    #[test]
    fn compose_for_general_is_empty_by_default() {
        let reg = PromptTemplateRegistry::with_defaults();
        let composed = reg.compose_for_context(ContextType::General);
        assert!(composed.is_empty());
    }

    #[test]
    fn templates_sorted_by_priority() {
        let mut reg = PromptTemplateRegistry::new();
        reg.add_template(PromptTemplate {
            name: "high".into(),
            content: "High priority".into(),
            priority: 20,
        });
        reg.add_template(PromptTemplate {
            name: "low".into(),
            content: "Low priority".into(),
            priority: 5,
        });
        reg.bind(ContextType::General, "high");
        reg.bind(ContextType::General, "low");

        let templates = reg.templates_for_context(ContextType::General);
        assert_eq!(templates[0].name, "low");
        assert_eq!(templates[1].name, "high");
    }

    // ── Tool selector ───────────────────────────────────────────────

    #[test]
    fn tool_selector_new_is_empty() {
        let sel = ToolSelector::new();
        assert!(sel.tools_for_context(ContextType::Debugging).is_empty());
    }

    #[test]
    fn tool_selector_with_defaults() {
        let sel = ToolSelector::with_defaults();
        let debug_tools = sel.tools_for_context(ContextType::Debugging);
        assert!(debug_tools.contains(&"read".to_string()));
        assert!(debug_tools.contains(&"grep".to_string()));
        assert!(debug_tools.contains(&"bash".to_string()));
    }

    #[test]
    fn tool_selector_register_and_query() {
        let mut sel = ToolSelector::new();
        sel.register(ContextType::Testing, &["bash", "read"]);

        assert!(sel.is_recommended(ContextType::Testing, "bash"));
        assert!(sel.is_recommended(ContextType::Testing, "read"));
        assert!(!sel.is_recommended(ContextType::Testing, "write"));
    }

    #[test]
    fn tool_selector_is_recommended_unknown_context() {
        let sel = ToolSelector::new();
        assert!(!sel.is_recommended(ContextType::General, "bash"));
    }

    #[test]
    fn tool_selector_navigation_tools() {
        let sel = ToolSelector::with_defaults();
        let nav_tools = sel.tools_for_context(ContextType::Navigation);
        assert!(nav_tools.contains(&"glob".to_string()));
        assert!(nav_tools.contains(&"grep".to_string()));
        assert!(!nav_tools.contains(&"write".to_string()));
    }

    // ── Tool recommender ────────────────────────────────────────────

    #[test]
    fn recommender_new_is_empty() {
        let rec = ToolRecommender::new();
        assert_eq!(rec.total_usages(), 0);
        assert!(rec.top_tools(5).is_empty());
    }

    #[test]
    fn recommender_record_and_top_tools() {
        let mut rec = ToolRecommender::new();
        rec.record_usage("read");
        rec.record_usage("read");
        rec.record_usage("read");
        rec.record_usage("grep");
        rec.record_usage("grep");
        rec.record_usage("bash");

        assert_eq!(rec.total_usages(), 6);

        let top = rec.top_tools(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "read");
        assert_eq!(top[0].1, 3);
        assert_eq!(top[1].0, "grep");
        assert_eq!(top[1].1, 2);
    }

    #[test]
    fn recommender_context_usage() {
        let mut rec = ToolRecommender::new();
        rec.record_context_usage(ContextType::Debugging, "grep");
        rec.record_context_usage(ContextType::Debugging, "grep");
        rec.record_context_usage(ContextType::Debugging, "read");
        rec.record_context_usage(ContextType::Testing, "bash");

        let debug_top = rec.top_tools_for_context(ContextType::Debugging, 5);
        assert_eq!(debug_top.len(), 2);
        assert_eq!(debug_top[0].0, "grep");
        assert_eq!(debug_top[0].1, 2);

        let test_top = rec.top_tools_for_context(ContextType::Testing, 5);
        assert_eq!(test_top.len(), 1);
        assert_eq!(test_top[0].0, "bash");
    }

    #[test]
    fn recommender_co_occurrence() {
        let mut rec = ToolRecommender::new();
        rec.record_co_occurrence(&["grep", "read"]);
        rec.record_co_occurrence(&["grep", "read"]);
        rec.record_co_occurrence(&["grep", "edit"]);

        let related = rec.related_tools("grep", 5);
        assert_eq!(related.len(), 2);
        assert_eq!(related[0].0, "read");
        assert_eq!(related[0].1, 2);
        assert_eq!(related[1].0, "edit");
        assert_eq!(related[1].1, 1);
    }

    #[test]
    fn recommender_co_occurrence_symmetric() {
        let mut rec = ToolRecommender::new();
        rec.record_co_occurrence(&["read", "grep"]);

        // Should be findable from both sides
        assert_eq!(rec.related_tools("read", 5).len(), 1);
        assert_eq!(rec.related_tools("grep", 5).len(), 1);
    }

    #[test]
    fn recommender_co_occurrence_three_tools() {
        let mut rec = ToolRecommender::new();
        rec.record_co_occurrence(&["a", "b", "c"]);

        // Should have 3 pairs: (a,b), (a,c), (b,c)
        assert_eq!(rec.related_tools("a", 5).len(), 2);
        assert_eq!(rec.related_tools("b", 5).len(), 2);
        assert_eq!(rec.related_tools("c", 5).len(), 2);
    }

    #[test]
    fn recommender_related_tools_no_data() {
        let rec = ToolRecommender::new();
        assert!(rec.related_tools("nonexistent", 5).is_empty());
    }

    #[test]
    fn recommender_top_tools_truncates() {
        let mut rec = ToolRecommender::new();
        for i in 0..10 {
            for _ in 0..=(10 - i) {
                rec.record_usage(&format!("tool_{i}"));
            }
        }
        let top = rec.top_tools(3);
        assert_eq!(top.len(), 3);
    }

    #[test]
    fn recommender_context_usage_empty_context() {
        let rec = ToolRecommender::new();
        assert!(
            rec.top_tools_for_context(ContextType::General, 5)
                .is_empty()
        );
    }
}
