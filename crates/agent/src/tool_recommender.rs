//! Context-aware tool recommendation: suggests the most likely useful tools
//! based on file types, conversation intent, and recent usage history.

use std::collections::HashMap;

// ── Data model ─────────────────────────────────────────────────────────

/// A recommended tool with a confidence score and justification.
#[derive(Debug, Clone)]
pub struct ToolRecommendation {
    pub tool_name: String,
    /// Confidence in the range `[0.0, 1.0]`.
    pub confidence: f64,
    pub reason: String,
}

/// Lightweight conversation context used for recommendation.
#[derive(Debug, Clone, Default)]
pub struct ConversationContext {
    /// The latest user message text.
    pub user_message: String,
    /// File paths mentioned or recently touched.
    pub active_files: Vec<String>,
    /// Tools used in the most recent N turns.
    pub recent_tools: Vec<String>,
}

// ── Intent detection ───────────────────────────────────────────────────

/// Coarse intent categories derived from the user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    FixBug,
    AddFeature,
    Refactor,
    Explore,
    Test,
    Build,
    Explain,
    Unknown,
}

/// Simple keyword-based intent detection.
#[must_use]
pub fn detect_intent(message: &str) -> Intent {
    let lower = message.to_lowercase();
    // Order matters: more specific patterns first.
    if lower.contains("fix")
        || lower.contains("bug")
        || lower.contains("error")
        || lower.contains("broken")
    {
        Intent::FixBug
    } else if lower.contains("test") || lower.contains("spec") || lower.contains("assert") {
        Intent::Test
    } else if lower.contains("refactor")
        || lower.contains("rename")
        || lower.contains("move")
        || lower.contains("clean")
    {
        Intent::Refactor
    } else if lower.contains("add")
        || lower.contains("implement")
        || lower.contains("create")
        || lower.contains("new")
    {
        Intent::AddFeature
    } else if lower.contains("build")
        || lower.contains("compile")
        || lower.contains("cargo")
        || lower.contains("npm")
    {
        Intent::Build
    } else if lower.contains("find")
        || lower.contains("search")
        || lower.contains("where")
        || lower.contains("show")
        || lower.contains("list")
    {
        Intent::Explore
    } else if lower.contains("explain")
        || lower.contains("what")
        || lower.contains("how")
        || lower.contains("why")
    {
        Intent::Explain
    } else {
        Intent::Unknown
    }
}

// ── Intent → tool mapping ──────────────────────────────────────────────

/// Tools recommended per intent with base confidence.
fn intent_tools(intent: Intent) -> Vec<(&'static str, f64, &'static str)> {
    match intent {
        Intent::FixBug => vec![
            ("read", 0.9, "read source to locate bug"),
            ("edit", 0.85, "apply the fix"),
            ("bash", 0.8, "run tests to verify"),
            ("grep", 0.7, "search for related occurrences"),
        ],
        Intent::AddFeature => vec![
            ("read", 0.85, "understand existing code"),
            ("write", 0.8, "create new files"),
            ("edit", 0.8, "modify existing files"),
            ("bash", 0.7, "run build/tests"),
        ],
        Intent::Refactor => vec![
            ("read", 0.9, "understand current structure"),
            ("edit", 0.9, "apply refactoring changes"),
            ("grep", 0.8, "find all references"),
            ("bash", 0.6, "run tests after refactor"),
        ],
        Intent::Explore => vec![
            ("read", 0.9, "read files of interest"),
            ("glob", 0.85, "find files by pattern"),
            ("grep", 0.85, "search content"),
        ],
        Intent::Test => vec![
            ("bash", 0.9, "run test suite"),
            ("read", 0.8, "read test files"),
            ("edit", 0.7, "fix or add tests"),
        ],
        Intent::Build => vec![
            ("bash", 0.95, "run build command"),
            ("read", 0.5, "check build config"),
        ],
        Intent::Explain => vec![
            ("read", 0.9, "read the code to explain"),
            ("grep", 0.6, "find related code"),
        ],
        Intent::Unknown => vec![
            ("read", 0.5, "general exploration"),
            ("bash", 0.4, "general commands"),
        ],
    }
}

// ── File-type hints ────────────────────────────────────────────────────

/// Additional tool hints based on file extensions present in context.
fn file_type_hints(files: &[String]) -> Vec<(&'static str, f64, String)> {
    let mut hints = Vec::new();
    let exts: Vec<&str> = files.iter().filter_map(|f| f.rsplit('.').next()).collect();

    if exts.iter().any(|e| *e == "rs" || *e == "toml") {
        hints.push(("bash", 0.6, "cargo build/test for Rust project".to_string()));
    }
    if exts
        .iter()
        .any(|e| *e == "js" || *e == "ts" || *e == "jsx" || *e == "tsx")
    {
        hints.push((
            "bash",
            0.6,
            "npm/yarn scripts for JS/TS project".to_string(),
        ));
    }
    if exts.contains(&"py") {
        hints.push(("bash", 0.6, "pytest/python for Python project".to_string()));
    }
    if exts.contains(&"go") {
        hints.push(("bash", 0.6, "go build/test for Go project".to_string()));
    }
    if exts.iter().any(|e| *e == "md" || *e == "txt") {
        hints.push(("read", 0.5, "read documentation files".to_string()));
    }
    hints
}

// ── History boost ──────────────────────────────────────────────────────

/// Boost confidence for tools recently used (recency bias).
fn history_boost(recent_tools: &[String]) -> HashMap<String, f64> {
    let mut boost: HashMap<String, f64> = HashMap::new();
    let len = recent_tools.len();
    for (i, name) in recent_tools.iter().enumerate() {
        // More recent → higher boost, max 0.15
        #[allow(clippy::cast_precision_loss)]
        let recency = (i + 1) as f64 / len.max(1) as f64;
        let b = 0.15 * recency;
        let entry = boost.entry(name.clone()).or_insert(0.0);
        if b > *entry {
            *entry = b;
        }
    }
    boost
}

// ── Recommender ────────────────────────────────────────────────────────

/// Produces tool recommendations by combining intent, file-type, and
/// history signals.
#[derive(Debug, Clone, Default)]
pub struct ContextToolRecommender {
    /// Maximum number of recommendations to return.
    pub max_recommendations: usize,
}

impl ContextToolRecommender {
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_recommendations: 5,
        }
    }

    /// Recommend tools for the given conversation context.
    #[must_use]
    pub fn recommend(&self, ctx: &ConversationContext) -> Vec<ToolRecommendation> {
        let intent = detect_intent(&ctx.user_message);
        let mut scores: HashMap<String, (f64, String)> = HashMap::new();

        // Intent-based recommendations.
        for (name, conf, reason) in intent_tools(intent) {
            let entry = scores
                .entry(name.to_string())
                .or_insert((0.0, String::new()));
            if conf > entry.0 {
                *entry = (conf, reason.to_string());
            }
        }

        // File-type hints.
        for (name, conf, reason) in file_type_hints(&ctx.active_files) {
            let entry = scores
                .entry(name.to_string())
                .or_insert((0.0, String::new()));
            if conf > entry.0 {
                *entry = (conf, reason);
            }
        }

        // History boost.
        let boost = history_boost(&ctx.recent_tools);
        for (name, b) in &boost {
            if let Some(entry) = scores.get_mut(name) {
                entry.0 = (entry.0 + b).min(1.0);
            }
        }

        let mut recs: Vec<ToolRecommendation> = scores
            .into_iter()
            .map(|(name, (conf, reason))| ToolRecommendation {
                tool_name: name,
                confidence: conf,
                reason,
            })
            .collect();
        recs.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        recs.truncate(self.max_recommendations);
        recs
    }
}

/// Convenience function: recommend tools for a context.
#[must_use]
pub fn recommend_tools(ctx: &ConversationContext) -> Vec<ToolRecommendation> {
    ContextToolRecommender::new().recommend(ctx)
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_fix_bug_intent() {
        assert_eq!(detect_intent("fix the bug in main.rs"), Intent::FixBug);
        assert_eq!(detect_intent("there's an error"), Intent::FixBug);
    }

    #[test]
    fn detect_add_feature_intent() {
        assert_eq!(
            detect_intent("add a new logging module"),
            Intent::AddFeature
        );
        assert_eq!(detect_intent("implement caching"), Intent::AddFeature);
    }

    #[test]
    fn detect_refactor_intent() {
        assert_eq!(detect_intent("refactor the auth module"), Intent::Refactor);
        assert_eq!(detect_intent("rename the function"), Intent::Refactor);
    }

    #[test]
    fn detect_explore_intent() {
        assert_eq!(detect_intent("find all uses of Foo"), Intent::Explore);
        assert_eq!(detect_intent("search for the config"), Intent::Explore);
    }

    #[test]
    fn detect_test_intent() {
        assert_eq!(detect_intent("run the tests"), Intent::Test);
        assert_eq!(
            detect_intent("add an assertion for this case"),
            Intent::Test
        );
    }

    #[test]
    fn detect_build_intent() {
        assert_eq!(detect_intent("build the project"), Intent::Build);
        assert_eq!(detect_intent("run cargo check"), Intent::Build);
    }

    #[test]
    fn detect_explain_intent() {
        assert_eq!(detect_intent("explain this function"), Intent::Explain);
        assert_eq!(detect_intent("what does this do"), Intent::Explain);
    }

    #[test]
    fn detect_unknown_intent() {
        assert_eq!(detect_intent("hello"), Intent::Unknown);
    }

    #[test]
    fn fix_bug_recommends_read_edit_bash() {
        let ctx = ConversationContext {
            user_message: "fix the bug in parser.rs".into(),
            ..Default::default()
        };
        let recs = recommend_tools(&ctx);
        let names: Vec<&str> = recs.iter().map(|r| r.tool_name.as_str()).collect();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"edit"));
        assert!(names.contains(&"bash"));
    }

    #[test]
    fn explore_recommends_glob_grep() {
        let ctx = ConversationContext {
            user_message: "find all error handlers".into(),
            ..Default::default()
        };
        // "find" → Explore, but "error" → FixBug (FixBug wins, earlier in order)
        // Try a pure explore intent
        let ctx2 = ConversationContext {
            user_message: "show me the project structure".into(),
            ..Default::default()
        };
        let recs = recommend_tools(&ctx2);
        let names: Vec<&str> = recs.iter().map(|r| r.tool_name.as_str()).collect();
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"grep"));
    }

    #[test]
    fn file_type_rust_boosts_bash() {
        let ctx = ConversationContext {
            user_message: "hello".into(), // Unknown intent → low confidence
            active_files: vec!["src/main.rs".into()],
            ..Default::default()
        };
        let recs = recommend_tools(&ctx);
        let bash = recs.iter().find(|r| r.tool_name == "bash");
        assert!(bash.is_some());
        assert!(bash.unwrap().confidence >= 0.6);
    }

    #[test]
    fn file_type_js_boosts_bash() {
        let ctx = ConversationContext {
            user_message: "hello".into(),
            active_files: vec!["index.tsx".into()],
            ..Default::default()
        };
        let recs = recommend_tools(&ctx);
        let bash = recs.iter().find(|r| r.tool_name == "bash");
        assert!(bash.is_some());
    }

    #[test]
    fn history_boost_increases_confidence() {
        let ctx_no_history = ConversationContext {
            user_message: "explain this code".into(),
            ..Default::default()
        };
        let ctx_with_history = ConversationContext {
            user_message: "explain this code".into(),
            recent_tools: vec!["grep".into(), "read".into()],
            ..Default::default()
        };
        let recs1 = recommend_tools(&ctx_no_history);
        let recs2 = recommend_tools(&ctx_with_history);
        let read1 = recs1
            .iter()
            .find(|r| r.tool_name == "read")
            .unwrap()
            .confidence;
        let read2 = recs2
            .iter()
            .find(|r| r.tool_name == "read")
            .unwrap()
            .confidence;
        assert!(read2 >= read1);
    }

    #[test]
    fn max_recommendations_respected() {
        let mut rec = ContextToolRecommender::new();
        rec.max_recommendations = 2;
        let ctx = ConversationContext {
            user_message: "fix bug in parser".into(),
            ..Default::default()
        };
        let recs = rec.recommend(&ctx);
        assert!(recs.len() <= 2);
    }

    #[test]
    fn recommendations_sorted_by_confidence() {
        let ctx = ConversationContext {
            user_message: "refactor the module".into(),
            ..Default::default()
        };
        let recs = recommend_tools(&ctx);
        for w in recs.windows(2) {
            assert!(w[0].confidence >= w[1].confidence);
        }
    }

    #[test]
    fn confidence_capped_at_one() {
        let ctx = ConversationContext {
            user_message: "run cargo build".into(), // Build intent, bash=0.95
            active_files: vec!["Cargo.toml".into()], // .toml → bash boost
            recent_tools: vec!["bash".into(), "bash".into(), "bash".into()],
            ..Default::default()
        };
        let recs = recommend_tools(&ctx);
        for r in &recs {
            assert!(r.confidence <= 1.0);
        }
    }

    #[test]
    fn recommendation_has_reason() {
        let ctx = ConversationContext {
            user_message: "fix the error".into(),
            ..Default::default()
        };
        let recs = recommend_tools(&ctx);
        for r in &recs {
            assert!(!r.reason.is_empty());
        }
    }

    #[test]
    fn empty_context() {
        let ctx = ConversationContext::default();
        let recs = recommend_tools(&ctx);
        // Should still produce some defaults (Unknown intent)
        assert!(!recs.is_empty());
    }

    #[test]
    fn default_recommender() {
        let r = ContextToolRecommender::default();
        assert_eq!(r.max_recommendations, 0);
        // new() sets 5
        let r2 = ContextToolRecommender::new();
        assert_eq!(r2.max_recommendations, 5);
    }
}
