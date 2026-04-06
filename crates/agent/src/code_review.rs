//! Code review assistant: rule-based review checklist and automated
//! diff analysis for common issues.

use std::collections::HashMap;

// ── Severity ──────────────────────────────────────────────────────────

/// Severity level for review findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

// ── Review item & checklist ───────────────────────────────────────────

/// A single review checklist item.
#[derive(Debug, Clone)]
pub struct ReviewItem {
    pub category: String,
    pub description: String,
    pub severity: Severity,
    /// Whether this item can be auto-checked by `auto_review`.
    pub auto_check: bool,
}

impl ReviewItem {
    #[must_use]
    pub fn new(
        category: impl Into<String>,
        description: impl Into<String>,
        severity: Severity,
        auto_check: bool,
    ) -> Self {
        Self {
            category: category.into(),
            description: description.into(),
            severity,
            auto_check,
        }
    }
}

/// A review checklist: a named collection of review items.
#[derive(Debug, Clone)]
pub struct ReviewChecklist {
    pub name: String,
    pub items: Vec<ReviewItem>,
}

impl ReviewChecklist {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            items: Vec::new(),
        }
    }

    /// Add a review item.
    pub fn add(&mut self, item: ReviewItem) {
        self.items.push(item);
    }

    /// Get items filtered by severity.
    #[must_use]
    pub fn by_severity(&self, severity: Severity) -> Vec<&ReviewItem> {
        self.items
            .iter()
            .filter(|i| i.severity == severity)
            .collect()
    }

    /// Get items that support auto-checking.
    #[must_use]
    pub fn auto_checkable(&self) -> Vec<&ReviewItem> {
        self.items.iter().filter(|i| i.auto_check).collect()
    }

    /// Number of items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Create a default checklist with common review items.
    #[must_use]
    pub fn default_checklist() -> Self {
        let mut cl = Self::new("default");
        cl.add(ReviewItem::new(
            "security",
            "Check for hardcoded secrets or API keys",
            Severity::Critical,
            true,
        ));
        cl.add(ReviewItem::new(
            "security",
            "Validate user input at system boundaries",
            Severity::Error,
            false,
        ));
        cl.add(ReviewItem::new(
            "error_handling",
            "Ensure errors are not silently swallowed",
            Severity::Warning,
            true,
        ));
        cl.add(ReviewItem::new(
            "error_handling",
            "Use proper error types instead of unwrap in library code",
            Severity::Warning,
            true,
        ));
        cl.add(ReviewItem::new(
            "style",
            "No TODO/FIXME/HACK without tracking issue",
            Severity::Info,
            true,
        ));
        cl.add(ReviewItem::new(
            "performance",
            "Avoid unnecessary allocations in hot paths",
            Severity::Warning,
            false,
        ));
        cl.add(ReviewItem::new(
            "testing",
            "New public API has test coverage",
            Severity::Warning,
            false,
        ));
        cl.add(ReviewItem::new(
            "documentation",
            "Public items have doc comments",
            Severity::Info,
            false,
        ));
        cl
    }
}

// ── Review finding ────────────────────────────────────────────────────

/// A finding produced by auto-review.
#[derive(Debug, Clone)]
pub struct ReviewFinding {
    pub line: usize,
    pub severity: Severity,
    pub category: String,
    pub message: String,
}

impl ReviewFinding {
    #[must_use]
    pub fn new(
        line: usize,
        severity: Severity,
        category: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            line,
            severity,
            category: category.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ReviewFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "L{}: [{}] ({}) {}",
            self.line, self.severity, self.category, self.message
        )
    }
}

// ── Auto-review rules ─────────────────────────────────────────────────

/// A rule-based auto-review engine.
#[derive(Debug, Clone, Default)]
pub struct AutoReviewer {
    rules: Vec<ReviewRule>,
}

/// A single review rule: pattern match + finding template.
#[derive(Debug, Clone)]
struct ReviewRule {
    pattern: String,
    severity: Severity,
    category: String,
    message: String,
}

impl AutoReviewer {
    #[must_use]
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Create a reviewer pre-loaded with built-in rules.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.add_rule("TODO", Severity::Info, "style", "TODO comment found");
        r.add_rule("FIXME", Severity::Warning, "style", "FIXME comment found");
        r.add_rule("HACK", Severity::Warning, "style", "HACK comment found");
        r.add_rule(
            ".unwrap()",
            Severity::Warning,
            "error_handling",
            "unwrap() may panic — consider proper error handling",
        );
        r.add_rule(
            ".expect(",
            Severity::Info,
            "error_handling",
            "expect() used — ensure panic message is descriptive",
        );
        r.add_rule(
            "panic!(",
            Severity::Warning,
            "error_handling",
            "Explicit panic — ensure this is intentional",
        );
        r.add_rule(
            "unsafe ",
            Severity::Warning,
            "safety",
            "Unsafe block — ensure invariants are documented",
        );
        r.add_rule(
            "unsafe{",
            Severity::Warning,
            "safety",
            "Unsafe block — ensure invariants are documented",
        );
        // Security patterns
        r.add_rule(
            "password",
            Severity::Warning,
            "security",
            "Possible hardcoded password reference",
        );
        r.add_rule(
            "secret",
            Severity::Warning,
            "security",
            "Possible hardcoded secret reference",
        );
        r.add_rule(
            "api_key",
            Severity::Critical,
            "security",
            "Possible hardcoded API key",
        );
        r.add_rule(
            "API_KEY",
            Severity::Critical,
            "security",
            "Possible hardcoded API key",
        );
        r
    }

    /// Add a custom rule.
    pub fn add_rule(&mut self, pattern: &str, severity: Severity, category: &str, message: &str) {
        self.rules.push(ReviewRule {
            pattern: pattern.to_string(),
            severity,
            category: category.to_string(),
            message: message.to_string(),
        });
    }

    /// Run auto-review on a diff or code string. Returns findings.
    #[must_use]
    pub fn review(&self, content: &str) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            let line_num = idx + 1;
            for rule in &self.rules {
                if line.contains(&rule.pattern) {
                    findings.push(ReviewFinding::new(
                        line_num,
                        rule.severity,
                        &rule.category,
                        &rule.message,
                    ));
                }
            }
        }
        findings
    }

    /// Number of rules.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

/// Convenience function: run auto-review with default rules.
#[must_use]
pub fn auto_review(diff: &str) -> Vec<ReviewFinding> {
    AutoReviewer::with_defaults().review(diff)
}

/// Summarize findings by severity.
#[must_use]
pub fn summarize_findings(findings: &[ReviewFinding]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for f in findings {
        *counts.entry(f.severity.to_string()).or_insert(0) += 1;
    }
    counts
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_display() {
        assert_eq!(Severity::Info.to_string(), "info");
        assert_eq!(Severity::Critical.to_string(), "critical");
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Critical);
    }

    #[test]
    fn review_item_new() {
        let item = ReviewItem::new("security", "check secrets", Severity::Critical, true);
        assert_eq!(item.category, "security");
        assert!(item.auto_check);
    }

    #[test]
    fn checklist_add_and_len() {
        let mut cl = ReviewChecklist::new("test");
        assert!(cl.is_empty());
        cl.add(ReviewItem::new("a", "b", Severity::Info, false));
        assert_eq!(cl.len(), 1);
    }

    #[test]
    fn checklist_by_severity() {
        let mut cl = ReviewChecklist::new("test");
        cl.add(ReviewItem::new("a", "low", Severity::Info, false));
        cl.add(ReviewItem::new("b", "high", Severity::Critical, false));
        cl.add(ReviewItem::new("c", "low2", Severity::Info, false));
        assert_eq!(cl.by_severity(Severity::Info).len(), 2);
        assert_eq!(cl.by_severity(Severity::Critical).len(), 1);
        assert_eq!(cl.by_severity(Severity::Warning).len(), 0);
    }

    #[test]
    fn checklist_auto_checkable() {
        let mut cl = ReviewChecklist::new("test");
        cl.add(ReviewItem::new("a", "auto", Severity::Info, true));
        cl.add(ReviewItem::new("b", "manual", Severity::Info, false));
        assert_eq!(cl.auto_checkable().len(), 1);
    }

    #[test]
    fn default_checklist_has_items() {
        let cl = ReviewChecklist::default_checklist();
        assert!(cl.len() >= 8);
        assert!(!cl.by_severity(Severity::Critical).is_empty());
    }

    #[test]
    fn finding_display() {
        let f = ReviewFinding::new(42, Severity::Warning, "style", "TODO found");
        let s = f.to_string();
        assert!(s.contains("L42"));
        assert!(s.contains("warning"));
        assert!(s.contains("style"));
    }

    #[test]
    fn auto_reviewer_empty() {
        let r = AutoReviewer::new();
        assert_eq!(r.rule_count(), 0);
        assert!(r.review("anything").is_empty());
    }

    #[test]
    fn auto_reviewer_defaults_has_rules() {
        let r = AutoReviewer::with_defaults();
        assert!(r.rule_count() >= 10);
    }

    #[test]
    fn auto_reviewer_detects_todo() {
        let r = AutoReviewer::with_defaults();
        let findings = r.review("// TODO: fix this later\nlet x = 1;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].severity, Severity::Info);
    }

    #[test]
    fn auto_reviewer_detects_unwrap() {
        let r = AutoReviewer::with_defaults();
        let findings = r.review("let val = result.unwrap();");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "error_handling");
    }

    #[test]
    fn auto_reviewer_detects_unsafe() {
        let r = AutoReviewer::with_defaults();
        let findings = r.review("unsafe { ptr::read(p) }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "safety");
    }

    #[test]
    fn auto_reviewer_detects_api_key() {
        let r = AutoReviewer::with_defaults();
        let findings = r.review("let API_KEY = \"abc123\";");
        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.severity == Severity::Critical));
    }

    #[test]
    fn auto_reviewer_multiple_findings_per_line() {
        let r = AutoReviewer::with_defaults();
        let findings = r.review("// TODO FIXME HACK");
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().all(|f| f.line == 1));
    }

    #[test]
    fn auto_reviewer_multi_line() {
        let r = AutoReviewer::with_defaults();
        let code = "fn main() {\n    let x = foo.unwrap();\n    // TODO cleanup\n}";
        let findings = r.review(code);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[1].line, 3);
    }

    #[test]
    fn auto_reviewer_custom_rule() {
        let mut r = AutoReviewer::new();
        r.add_rule(
            "dbg!",
            Severity::Warning,
            "debug",
            "Debug macro left in code",
        );
        let findings = r.review("dbg!(value);");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "debug");
    }

    #[test]
    fn auto_review_convenience() {
        let findings = auto_review("// FIXME: broken\nlet key = api_key;");
        assert!(findings.len() >= 2);
    }

    #[test]
    fn summarize_findings_counts() {
        let findings = vec![
            ReviewFinding::new(1, Severity::Warning, "a", "x"),
            ReviewFinding::new(2, Severity::Warning, "b", "y"),
            ReviewFinding::new(3, Severity::Critical, "c", "z"),
        ];
        let summary = summarize_findings(&findings);
        assert_eq!(summary.get("warning"), Some(&2));
        assert_eq!(summary.get("critical"), Some(&1));
    }

    #[test]
    fn summarize_empty() {
        let summary = summarize_findings(&[]);
        assert!(summary.is_empty());
    }

    #[test]
    fn finding_new() {
        let f = ReviewFinding::new(10, Severity::Error, "test", "msg");
        assert_eq!(f.line, 10);
        assert_eq!(f.severity, Severity::Error);
    }
}
