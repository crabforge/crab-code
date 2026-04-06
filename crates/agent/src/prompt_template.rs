//! Dynamic system prompt templates with variable substitution and
//! conditional sections.

use std::collections::HashMap;

// ── Template context ───────────────────────────────────────────────────

/// Runtime variables available during template rendering.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    vars: HashMap<String, String>,
}

impl TemplateContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a variable value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    /// Get a variable value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    /// Check whether a variable is set and non-empty (truthy).
    #[must_use]
    pub fn is_truthy(&self, key: &str) -> bool {
        self.vars
            .get(key)
            .is_some_and(|v| !v.is_empty() && v != "false" && v != "0")
    }

    /// Number of variables.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Whether the context has no variables.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

// ── Template engine ────────────────────────────────────────────────────

/// A prompt template with `{{variable}}` placeholders and
/// `{{#if var}}...{{/if}}` conditional blocks.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub template: String,
}

impl PromptTemplate {
    #[must_use]
    pub fn new(name: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            template: template.into(),
        }
    }
}

/// Renders `PromptTemplate`s against a `TemplateContext`.
#[derive(Debug, Clone, Default)]
pub struct TemplateEngine;

impl TemplateEngine {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Render a template string with the given context.
    ///
    /// Supports:
    /// - `{{variable}}` — replaced by context value, or empty string if missing
    /// - `{{#if variable}}...{{/if}}` — included only if variable is truthy
    /// - `{{#unless variable}}...{{/unless}}` — included only if variable is falsy
    #[must_use]
    pub fn render(&self, template: &str, ctx: &TemplateContext) -> String {
        let mut result = template.to_string();

        // Process {{#if var}}...{{/if}} blocks (non-nested).
        result = self.process_conditionals(&result, ctx);

        // Process {{#unless var}}...{{/unless}} blocks.
        result = self.process_unless(&result, ctx);

        // Process {{variable}} substitutions.
        result = self.process_variables(&result, ctx);

        result
    }

    /// Render a `PromptTemplate`.
    #[must_use]
    pub fn render_template(&self, tmpl: &PromptTemplate, ctx: &TemplateContext) -> String {
        self.render(&tmpl.template, ctx)
    }

    #[allow(clippy::unused_self)]
    fn process_conditionals(&self, input: &str, ctx: &TemplateContext) -> String {
        let mut result = input.to_string();
        // Simple non-nested {{#if var}}...{{/if}}
        while let Some(start) = result.find("{{#if ") {
            let Some(tag_end) = result[start..].find("}}") else {
                break;
            };
            let var_name = &result[start + 6..start + tag_end].trim();
            let close_tag = "{{/if}}";
            let Some(close_pos) = result[start + tag_end + 2..].find(close_tag) else {
                break;
            };
            let body_start = start + tag_end + 2;
            let body_end = body_start + close_pos;
            let body = &result[body_start..body_end];
            let full_end = body_end + close_tag.len();

            if ctx.is_truthy(var_name) {
                result = format!("{}{}{}", &result[..start], body, &result[full_end..]);
            } else {
                result = format!("{}{}", &result[..start], &result[full_end..]);
            }
        }
        result
    }

    #[allow(clippy::unused_self)]
    fn process_unless(&self, input: &str, ctx: &TemplateContext) -> String {
        let mut result = input.to_string();
        while let Some(start) = result.find("{{#unless ") {
            let Some(tag_end) = result[start..].find("}}") else {
                break;
            };
            let var_name = &result[start + 10..start + tag_end].trim();
            let close_tag = "{{/unless}}";
            let Some(close_pos) = result[start + tag_end + 2..].find(close_tag) else {
                break;
            };
            let body_start = start + tag_end + 2;
            let body_end = body_start + close_pos;
            let body = &result[body_start..body_end];
            let full_end = body_end + close_tag.len();

            if ctx.is_truthy(var_name) {
                result = format!("{}{}", &result[..start], &result[full_end..]);
            } else {
                result = format!("{}{}{}", &result[..start], body, &result[full_end..]);
            }
        }
        result
    }

    #[allow(clippy::unused_self)]
    fn process_variables(&self, input: &str, ctx: &TemplateContext) -> String {
        let mut result = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            // Skip conditional tags that might remain
            if rest[start..].starts_with("{{#") || rest[start..].starts_with("{{/") {
                result.push_str(&rest[..start + 2]);
                rest = &rest[start + 2..];
                continue;
            }
            result.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            if let Some(end) = after.find("}}") {
                let var_name = after[..end].trim();
                if let Some(val) = ctx.get(var_name) {
                    result.push_str(val);
                }
                rest = &after[end + 2..];
            } else {
                result.push_str("{{");
                rest = after;
            }
        }
        result.push_str(rest);
        result
    }
}

// ── Built-in templates ─────────────────────────────────────────────────

/// A library of pre-defined prompt templates.
#[derive(Debug, Clone, Default)]
pub struct BuiltinTemplates {
    templates: HashMap<String, PromptTemplate>,
}

impl BuiltinTemplates {
    /// Create the standard built-in template set.
    #[must_use]
    pub fn new() -> Self {
        let mut templates = HashMap::new();

        templates.insert(
            "coding".into(),
            PromptTemplate::new(
                "coding",
                "You are an expert software engineer.\n\
                 {{#if project_name}}Project: {{project_name}}\n{{/if}}\
                 {{#if language}}Primary language: {{language}}\n{{/if}}\
                 {{#if cwd}}Working directory: {{cwd}}\n{{/if}}\
                 {{#if git_status}}Git status: {{git_status}}\n{{/if}}\
                 Write clean, idiomatic code. Follow existing conventions.",
            ),
        );

        templates.insert(
            "debugging".into(),
            PromptTemplate::new(
                "debugging",
                "You are a debugging specialist.\n\
                 {{#if error_message}}Error: {{error_message}}\n{{/if}}\
                 {{#if stack_trace}}Stack trace:\n{{stack_trace}}\n{{/if}}\
                 Approach:\n\
                 1. Reproduce the issue\n\
                 2. Identify root cause\n\
                 3. Propose minimal fix\n\
                 4. Verify the fix",
            ),
        );

        templates.insert(
            "review".into(),
            PromptTemplate::new(
                "review",
                "You are a code reviewer.\n\
                 {{#if project_name}}Project: {{project_name}}\n{{/if}}\
                 Check for:\n\
                 - Correctness and edge cases\n\
                 - Security vulnerabilities\n\
                 - Performance issues\n\
                 - Code clarity and maintainability\n\
                 {{#if language}}- {{language}} idioms and best practices\n{{/if}}\
                 Be constructive and specific.",
            ),
        );

        templates.insert(
            "explain".into(),
            PromptTemplate::new(
                "explain",
                "You are a patient technical explainer.\n\
                 {{#if language}}Language: {{language}}\n{{/if}}\
                 {{#if audience}}Audience level: {{audience}}\n{{/if}}\
                 {{#unless audience}}Explain clearly for a developer audience.\n{{/unless}}\
                 Use examples where helpful.",
            ),
        );

        Self { templates }
    }

    /// Get a template by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PromptTemplate> {
        self.templates.get(name)
    }

    /// List all template names.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.templates.keys().map(String::as_str).collect()
    }

    /// Number of built-in templates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// Whether the library is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Register a custom template.
    pub fn register(&mut self, template: PromptTemplate) {
        self.templates.insert(template.name.clone(), template);
    }

    /// Render a named template with the given context.
    #[must_use]
    pub fn render(&self, name: &str, ctx: &TemplateContext) -> Option<String> {
        let engine = TemplateEngine::new();
        self.get(name).map(|t| engine.render_template(t, ctx))
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_set_and_get() {
        let mut ctx = TemplateContext::new();
        ctx.set("name", "test");
        assert_eq!(ctx.get("name"), Some("test"));
        assert_eq!(ctx.get("missing"), None);
    }

    #[test]
    fn context_truthy() {
        let mut ctx = TemplateContext::new();
        ctx.set("a", "hello");
        ctx.set("b", "");
        ctx.set("c", "false");
        ctx.set("d", "0");
        assert!(ctx.is_truthy("a"));
        assert!(!ctx.is_truthy("b"));
        assert!(!ctx.is_truthy("c"));
        assert!(!ctx.is_truthy("d"));
        assert!(!ctx.is_truthy("missing"));
    }

    #[test]
    fn context_len() {
        let mut ctx = TemplateContext::new();
        assert!(ctx.is_empty());
        ctx.set("k", "v");
        assert_eq!(ctx.len(), 1);
    }

    #[test]
    fn simple_variable_substitution() {
        let engine = TemplateEngine::new();
        let ctx = {
            let mut c = TemplateContext::new();
            c.set("name", "Rust");
            c
        };
        let result = engine.render("Hello {{name}}!", &ctx);
        assert_eq!(result, "Hello Rust!");
    }

    #[test]
    fn missing_variable_replaced_with_empty() {
        let engine = TemplateEngine::new();
        let ctx = TemplateContext::new();
        let result = engine.render("Hello {{name}}!", &ctx);
        assert_eq!(result, "Hello !");
    }

    #[test]
    fn multiple_variables() {
        let engine = TemplateEngine::new();
        let mut ctx = TemplateContext::new();
        ctx.set("a", "1");
        ctx.set("b", "2");
        let result = engine.render("{{a}} + {{b}} = 3", &ctx);
        assert_eq!(result, "1 + 2 = 3");
    }

    #[test]
    fn if_block_truthy() {
        let engine = TemplateEngine::new();
        let mut ctx = TemplateContext::new();
        ctx.set("show", "yes");
        let result = engine.render("before{{#if show}} visible{{/if}} after", &ctx);
        assert_eq!(result, "before visible after");
    }

    #[test]
    fn if_block_falsy() {
        let engine = TemplateEngine::new();
        let ctx = TemplateContext::new();
        let result = engine.render("before{{#if show}} hidden{{/if}} after", &ctx);
        assert_eq!(result, "before after");
    }

    #[test]
    fn unless_block_falsy() {
        let engine = TemplateEngine::new();
        let ctx = TemplateContext::new();
        let result = engine.render("{{#unless auth}}no auth{{/unless}}", &ctx);
        assert_eq!(result, "no auth");
    }

    #[test]
    fn unless_block_truthy() {
        let engine = TemplateEngine::new();
        let mut ctx = TemplateContext::new();
        ctx.set("auth", "yes");
        let result = engine.render("{{#unless auth}}no auth{{/unless}}", &ctx);
        assert_eq!(result, "");
    }

    #[test]
    fn if_block_with_variable() {
        let engine = TemplateEngine::new();
        let mut ctx = TemplateContext::new();
        ctx.set("lang", "Rust");
        let result = engine.render("{{#if lang}}Language: {{lang}}{{/if}}", &ctx);
        assert_eq!(result, "Language: Rust");
    }

    #[test]
    fn no_templates_empty() {
        let engine = TemplateEngine::new();
        let ctx = TemplateContext::new();
        let result = engine.render("plain text", &ctx);
        assert_eq!(result, "plain text");
    }

    #[test]
    fn unclosed_variable_tag() {
        let engine = TemplateEngine::new();
        let ctx = TemplateContext::new();
        let result = engine.render("hello {{name", &ctx);
        assert_eq!(result, "hello {{name");
    }

    #[test]
    fn prompt_template_new() {
        let t = PromptTemplate::new("test", "content {{var}}");
        assert_eq!(t.name, "test");
        assert_eq!(t.template, "content {{var}}");
    }

    #[test]
    fn render_template() {
        let engine = TemplateEngine::new();
        let tmpl = PromptTemplate::new("t", "Hello {{who}}");
        let mut ctx = TemplateContext::new();
        ctx.set("who", "world");
        assert_eq!(engine.render_template(&tmpl, &ctx), "Hello world");
    }

    #[test]
    fn builtin_templates_exist() {
        let builtins = BuiltinTemplates::new();
        assert_eq!(builtins.len(), 4);
        assert!(!builtins.is_empty());
        assert!(builtins.get("coding").is_some());
        assert!(builtins.get("debugging").is_some());
        assert!(builtins.get("review").is_some());
        assert!(builtins.get("explain").is_some());
    }

    #[test]
    fn builtin_names() {
        let builtins = BuiltinTemplates::new();
        let names = builtins.names();
        assert!(names.contains(&"coding"));
        assert!(names.contains(&"debugging"));
    }

    #[test]
    fn builtin_render_coding() {
        let builtins = BuiltinTemplates::new();
        let mut ctx = TemplateContext::new();
        ctx.set("project_name", "crab-code");
        ctx.set("language", "Rust");
        let rendered = builtins.render("coding", &ctx).unwrap();
        assert!(rendered.contains("crab-code"));
        assert!(rendered.contains("Rust"));
        assert!(rendered.contains("expert software engineer"));
    }

    #[test]
    fn builtin_render_debugging() {
        let builtins = BuiltinTemplates::new();
        let mut ctx = TemplateContext::new();
        ctx.set("error_message", "index out of bounds");
        let rendered = builtins.render("debugging", &ctx).unwrap();
        assert!(rendered.contains("index out of bounds"));
        assert!(rendered.contains("Reproduce"));
    }

    #[test]
    fn builtin_render_review_without_lang() {
        let builtins = BuiltinTemplates::new();
        let ctx = TemplateContext::new();
        let rendered = builtins.render("review", &ctx).unwrap();
        assert!(rendered.contains("code reviewer"));
        assert!(!rendered.contains("idioms"));
    }

    #[test]
    fn builtin_render_explain_with_unless() {
        let builtins = BuiltinTemplates::new();
        let ctx = TemplateContext::new();
        let rendered = builtins.render("explain", &ctx).unwrap();
        assert!(rendered.contains("Explain clearly"));
    }

    #[test]
    fn builtin_render_explain_with_audience() {
        let builtins = BuiltinTemplates::new();
        let mut ctx = TemplateContext::new();
        ctx.set("audience", "beginner");
        let rendered = builtins.render("explain", &ctx).unwrap();
        assert!(rendered.contains("beginner"));
        assert!(!rendered.contains("Explain clearly"));
    }

    #[test]
    fn builtin_render_missing() {
        let builtins = BuiltinTemplates::new();
        let ctx = TemplateContext::new();
        assert!(builtins.render("nonexistent", &ctx).is_none());
    }

    #[test]
    fn register_custom_template() {
        let mut builtins = BuiltinTemplates::new();
        builtins.register(PromptTemplate::new("custom", "my template"));
        assert_eq!(builtins.len(), 5);
        assert!(builtins.get("custom").is_some());
    }

    #[test]
    fn default_builtin_templates() {
        let builtins = BuiltinTemplates::default();
        assert!(builtins.is_empty());
    }

    #[test]
    fn whitespace_in_variable_name() {
        let engine = TemplateEngine::new();
        let mut ctx = TemplateContext::new();
        ctx.set("name", "test");
        let result = engine.render("{{ name }}", &ctx);
        assert_eq!(result, "test");
    }

    #[test]
    fn multiple_if_blocks() {
        let engine = TemplateEngine::new();
        let mut ctx = TemplateContext::new();
        ctx.set("a", "1");
        let tmpl = "{{#if a}}A{{/if}} {{#if b}}B{{/if}} {{#if a}}C{{/if}}";
        let result = engine.render(tmpl, &ctx);
        assert_eq!(result, "A  C");
    }
}
