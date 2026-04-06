//! Context-aware prompt optimization: dynamically adjusts the system prompt
//! by adding, removing, or reordering sections based on conversation context.

// ── Prompt sections ────────────────────────────────────────────────────

/// Priority level for a prompt section — higher priority sections are
/// retained when the prompt must be trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionPriority {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3,
}

/// A conditional section of a system prompt.
#[derive(Debug, Clone)]
pub struct PromptSection {
    pub name: String,
    pub content: String,
    pub priority: SectionPriority,
    /// If set, the section is only included when this condition matches.
    pub condition: Option<SectionCondition>,
}

/// Condition under which a section is included.
#[derive(Debug, Clone)]
pub enum SectionCondition {
    /// Include when conversation turn count <= threshold (short conversations).
    ShortConversation(usize),
    /// Include when conversation turn count > threshold (long conversations).
    LongConversation(usize),
    /// Include when the scenario matches.
    Scenario(PromptScenario),
    /// Always include.
    Always,
}

/// High-level scenario detected from conversation context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptScenario {
    Coding,
    Debugging,
    CodeReview,
    Explaining,
    General,
}

// ── Optimization context ───────────────────────────────────────────────

/// Context information used to decide which sections to include.
#[derive(Debug, Clone, Default)]
pub struct OptimizationContext {
    /// Number of conversation turns so far.
    pub turn_count: usize,
    /// Detected scenario.
    pub scenario: Option<PromptScenario>,
    /// Available token budget for the system prompt (0 = unlimited).
    pub token_budget: usize,
    /// Whether errors have occurred recently.
    pub has_recent_errors: bool,
    /// Whether code changes have been made.
    pub has_code_changes: bool,
}

/// The result of prompt optimization.
#[derive(Debug, Clone)]
pub struct OptimizedPrompt {
    /// The final assembled prompt text.
    pub text: String,
    /// Sections that were included.
    pub included_sections: Vec<String>,
    /// Sections that were excluded.
    pub excluded_sections: Vec<String>,
    /// Estimated token count (chars / 4).
    pub estimated_tokens: usize,
}

// ── Optimizer ──────────────────────────────────────────────────────────

/// Optimizes a system prompt by selecting and ordering sections based
/// on conversation context.
#[derive(Debug, Clone, Default)]
pub struct PromptOptimizer {
    sections: Vec<PromptSection>,
}

impl PromptOptimizer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a section to the optimizer.
    pub fn add_section(&mut self, section: PromptSection) {
        self.sections.push(section);
    }

    /// Number of registered sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Build an optimized prompt from a base prompt and context.
    #[must_use]
    pub fn optimize(&self, base_prompt: &str, ctx: &OptimizationContext) -> OptimizedPrompt {
        let mut included = Vec::new();
        let mut excluded = Vec::new();

        // Evaluate conditions and collect eligible sections.
        let mut eligible: Vec<&PromptSection> = Vec::new();
        for section in &self.sections {
            if self.evaluate_condition(section, ctx) {
                eligible.push(section);
            } else {
                excluded.push(section.name.clone());
            }
        }

        // Sort by priority descending (Critical first).
        eligible.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Build the prompt text, respecting token budget.
        let mut parts: Vec<String> = vec![base_prompt.to_string()];
        let base_tokens = estimate_tokens(base_prompt);
        let mut total_tokens = base_tokens;
        let budget = if ctx.token_budget > 0 {
            ctx.token_budget
        } else {
            usize::MAX
        };

        for section in eligible {
            let section_tokens = estimate_tokens(&section.content);
            if total_tokens + section_tokens <= budget {
                parts.push(section.content.clone());
                included.push(section.name.clone());
                total_tokens += section_tokens;
            } else {
                excluded.push(section.name.clone());
            }
        }

        let text = parts.join("\n\n");
        let estimated_tokens = estimate_tokens(&text);

        OptimizedPrompt {
            text,
            included_sections: included,
            excluded_sections: excluded,
            estimated_tokens,
        }
    }

    #[allow(clippy::unused_self)]
    fn evaluate_condition(&self, section: &PromptSection, ctx: &OptimizationContext) -> bool {
        match &section.condition {
            None | Some(SectionCondition::Always) => true,
            Some(SectionCondition::ShortConversation(threshold)) => ctx.turn_count <= *threshold,
            Some(SectionCondition::LongConversation(threshold)) => ctx.turn_count > *threshold,
            Some(SectionCondition::Scenario(scenario)) => ctx.scenario.as_ref() == Some(scenario),
        }
    }
}

/// Create a standard set of optimizer sections for common scenarios.
#[must_use]
pub fn default_optimizer() -> PromptOptimizer {
    let mut opt = PromptOptimizer::new();

    opt.add_section(PromptSection {
        name: "core_identity".into(),
        content: "You are an expert AI coding assistant.".into(),
        priority: SectionPriority::Critical,
        condition: Some(SectionCondition::Always),
    });

    opt.add_section(PromptSection {
        name: "onboarding".into(),
        content: "Start by understanding the user's request. Ask clarifying questions if needed."
            .into(),
        priority: SectionPriority::Medium,
        condition: Some(SectionCondition::ShortConversation(3)),
    });

    opt.add_section(PromptSection {
        name: "debug_guidance".into(),
        content: "Debugging approach:\n\
                  1. Read the error message carefully\n\
                  2. Check assumptions against actual state\n\
                  3. Propose a focused fix — avoid unrelated changes\n\
                  4. Verify the fix with tests"
            .into(),
        priority: SectionPriority::High,
        condition: Some(SectionCondition::Scenario(PromptScenario::Debugging)),
    });

    opt.add_section(PromptSection {
        name: "review_checklist".into(),
        content: "Code review checklist:\n\
                  - Correctness: does the logic match the intent?\n\
                  - Security: any injection, auth bypass, or data leak?\n\
                  - Performance: unnecessary allocations or O(n^2)?\n\
                  - Style: consistent with codebase conventions?"
            .into(),
        priority: SectionPriority::High,
        condition: Some(SectionCondition::Scenario(PromptScenario::CodeReview)),
    });

    opt.add_section(PromptSection {
        name: "conciseness".into(),
        content: "Be concise. The user has context from prior turns.".into(),
        priority: SectionPriority::Low,
        condition: Some(SectionCondition::LongConversation(10)),
    });

    opt
}

/// Detect scenario from a user message (simple keyword heuristic).
#[must_use]
pub fn detect_scenario(message: &str) -> PromptScenario {
    let lower = message.to_lowercase();
    if lower.contains("bug")
        || lower.contains("error")
        || lower.contains("fix")
        || lower.contains("debug")
        || lower.contains("broken")
    {
        PromptScenario::Debugging
    } else if lower.contains("review") || lower.contains("check") || lower.contains("pr") {
        PromptScenario::CodeReview
    } else if lower.contains("explain")
        || lower.contains("what")
        || lower.contains("how")
        || lower.contains("why")
    {
        PromptScenario::Explaining
    } else if lower.contains("add")
        || lower.contains("implement")
        || lower.contains("create")
        || lower.contains("write")
        || lower.contains("build")
    {
        PromptScenario::Coding
    } else {
        PromptScenario::General
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_optimizer() {
        let opt = PromptOptimizer::new();
        assert_eq!(opt.section_count(), 0);
        let result = opt.optimize("base", &OptimizationContext::default());
        assert_eq!(result.text, "base");
        assert!(result.included_sections.is_empty());
    }

    #[test]
    fn always_section_included() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "always".into(),
            content: "always here".into(),
            priority: SectionPriority::Critical,
            condition: Some(SectionCondition::Always),
        });
        let result = opt.optimize("base", &OptimizationContext::default());
        assert!(result.text.contains("always here"));
        assert!(result.included_sections.contains(&"always".to_string()));
    }

    #[test]
    fn short_conversation_condition() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "intro".into(),
            content: "intro text".into(),
            priority: SectionPriority::Medium,
            condition: Some(SectionCondition::ShortConversation(5)),
        });

        let short_ctx = OptimizationContext {
            turn_count: 2,
            ..Default::default()
        };
        let result = opt.optimize("", &short_ctx);
        assert!(result.included_sections.contains(&"intro".to_string()));

        let long_ctx = OptimizationContext {
            turn_count: 10,
            ..Default::default()
        };
        let result = opt.optimize("", &long_ctx);
        assert!(result.excluded_sections.contains(&"intro".to_string()));
    }

    #[test]
    fn long_conversation_condition() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "concise".into(),
            content: "be brief".into(),
            priority: SectionPriority::Low,
            condition: Some(SectionCondition::LongConversation(5)),
        });

        let short_ctx = OptimizationContext {
            turn_count: 3,
            ..Default::default()
        };
        let result = opt.optimize("", &short_ctx);
        assert!(result.excluded_sections.contains(&"concise".to_string()));

        let long_ctx = OptimizationContext {
            turn_count: 10,
            ..Default::default()
        };
        let result = opt.optimize("", &long_ctx);
        assert!(result.included_sections.contains(&"concise".to_string()));
    }

    #[test]
    fn scenario_condition() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "debug".into(),
            content: "debug help".into(),
            priority: SectionPriority::High,
            condition: Some(SectionCondition::Scenario(PromptScenario::Debugging)),
        });

        let debug_ctx = OptimizationContext {
            scenario: Some(PromptScenario::Debugging),
            ..Default::default()
        };
        let result = opt.optimize("", &debug_ctx);
        assert!(result.included_sections.contains(&"debug".to_string()));

        let code_ctx = OptimizationContext {
            scenario: Some(PromptScenario::Coding),
            ..Default::default()
        };
        let result = opt.optimize("", &code_ctx);
        assert!(result.excluded_sections.contains(&"debug".to_string()));
    }

    #[test]
    fn no_condition_means_always() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "default".into(),
            content: "included".into(),
            priority: SectionPriority::Medium,
            condition: None,
        });
        let result = opt.optimize("", &OptimizationContext::default());
        assert!(result.included_sections.contains(&"default".to_string()));
    }

    #[test]
    fn priority_ordering() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "low".into(),
            content: "LOW".into(),
            priority: SectionPriority::Low,
            condition: None,
        });
        opt.add_section(PromptSection {
            name: "critical".into(),
            content: "CRITICAL".into(),
            priority: SectionPriority::Critical,
            condition: None,
        });
        let result = opt.optimize("BASE", &OptimizationContext::default());
        // Critical should come before Low in the output
        let crit_pos = result.text.find("CRITICAL").unwrap();
        let low_pos = result.text.find("LOW").unwrap();
        assert!(crit_pos < low_pos);
    }

    #[test]
    fn token_budget_trims_low_priority() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "critical".into(),
            content: "A".into(),
            priority: SectionPriority::Critical,
            condition: None,
        });
        opt.add_section(PromptSection {
            name: "low".into(),
            content: "B".repeat(1000),
            priority: SectionPriority::Low,
            condition: None,
        });
        let ctx = OptimizationContext {
            token_budget: 10, // Very tight
            ..Default::default()
        };
        let result = opt.optimize("hi", &ctx);
        assert!(result.included_sections.contains(&"critical".to_string()));
        assert!(result.excluded_sections.contains(&"low".to_string()));
    }

    #[test]
    fn zero_budget_means_unlimited() {
        let mut opt = PromptOptimizer::new();
        opt.add_section(PromptSection {
            name: "big".into(),
            content: "X".repeat(10000),
            priority: SectionPriority::Low,
            condition: None,
        });
        let ctx = OptimizationContext {
            token_budget: 0, // Unlimited
            ..Default::default()
        };
        let result = opt.optimize("", &ctx);
        assert!(result.included_sections.contains(&"big".to_string()));
    }

    #[test]
    fn estimated_tokens_in_result() {
        let opt = PromptOptimizer::new();
        let result = opt.optimize("hello world", &OptimizationContext::default());
        assert_eq!(result.estimated_tokens, "hello world".len() / 4);
    }

    #[test]
    fn default_optimizer_sections() {
        let opt = default_optimizer();
        assert!(opt.section_count() >= 4);
    }

    #[test]
    fn default_optimizer_debug_scenario() {
        let opt = default_optimizer();
        let ctx = OptimizationContext {
            scenario: Some(PromptScenario::Debugging),
            ..Default::default()
        };
        let result = opt.optimize("", &ctx);
        assert!(
            result
                .included_sections
                .contains(&"debug_guidance".to_string())
        );
        assert!(
            !result
                .included_sections
                .contains(&"review_checklist".to_string())
        );
    }

    #[test]
    fn default_optimizer_review_scenario() {
        let opt = default_optimizer();
        let ctx = OptimizationContext {
            scenario: Some(PromptScenario::CodeReview),
            ..Default::default()
        };
        let result = opt.optimize("", &ctx);
        assert!(
            result
                .included_sections
                .contains(&"review_checklist".to_string())
        );
        assert!(
            !result
                .included_sections
                .contains(&"debug_guidance".to_string())
        );
    }

    #[test]
    fn detect_scenario_debugging() {
        assert_eq!(detect_scenario("fix the bug"), PromptScenario::Debugging);
        assert_eq!(
            detect_scenario("there is an error"),
            PromptScenario::Debugging
        );
    }

    #[test]
    fn detect_scenario_review() {
        assert_eq!(
            detect_scenario("review this code"),
            PromptScenario::CodeReview
        );
        assert_eq!(detect_scenario("check the PR"), PromptScenario::CodeReview);
    }

    #[test]
    fn detect_scenario_explain() {
        assert_eq!(
            detect_scenario("explain this function"),
            PromptScenario::Explaining
        );
        assert_eq!(
            detect_scenario("how does this work"),
            PromptScenario::Explaining
        );
    }

    #[test]
    fn detect_scenario_coding() {
        assert_eq!(
            detect_scenario("implement a parser"),
            PromptScenario::Coding
        );
        assert_eq!(
            detect_scenario("add a new endpoint"),
            PromptScenario::Coding
        );
    }

    #[test]
    fn detect_scenario_general() {
        assert_eq!(detect_scenario("hello"), PromptScenario::General);
    }

    #[test]
    fn section_priority_ordering() {
        assert!(SectionPriority::Critical > SectionPriority::High);
        assert!(SectionPriority::High > SectionPriority::Medium);
        assert!(SectionPriority::Medium > SectionPriority::Low);
    }
}
