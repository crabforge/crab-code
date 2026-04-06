//! Code generation templates: parameterized code templates for common
//! patterns across multiple languages.

use std::collections::HashMap;

// ── Language detection ─────────────────────────────────────────────────

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Unknown,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "rust"),
            Self::TypeScript => write!(f, "typescript"),
            Self::JavaScript => write!(f, "javascript"),
            Self::Python => write!(f, "python"),
            Self::Go => write!(f, "go"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Detect language from a file extension.
#[must_use]
pub fn detect_language(path: &str) -> Language {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => Language::Rust,
        "ts" | "tsx" => Language::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
        "py" | "pyi" => Language::Python,
        "go" => Language::Go,
        _ => Language::Unknown,
    }
}

// ── Code template ──────────────────────────────────────────────────────

/// A parameterized code template.
#[derive(Debug, Clone)]
pub struct CodeTemplate {
    pub name: String,
    pub language: Language,
    pub template: String,
    pub variables: Vec<String>,
}

impl CodeTemplate {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        language: Language,
        template: impl Into<String>,
        variables: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            language,
            template: template.into(),
            variables,
        }
    }

    /// Render the template with the given variable values.
    #[must_use]
    pub fn render(&self, vars: &HashMap<String, String>) -> String {
        let mut result = self.template.clone();
        for key in &self.variables {
            let placeholder = format!("{{{{{key}}}}}");
            if let Some(val) = vars.get(key) {
                result = result.replace(&placeholder, val);
            }
        }
        result
    }
}

// ── Template library ───────────────────────────────────────────────────

/// Registry of code templates.
#[derive(Debug, Clone, Default)]
pub struct TemplateLibrary {
    templates: HashMap<String, CodeTemplate>,
}

impl TemplateLibrary {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a library pre-loaded with built-in templates.
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut lib = Self::new();
        lib.register_builtins();
        lib
    }

    /// Register a template.
    pub fn register(&mut self, template: CodeTemplate) {
        self.templates.insert(template.name.clone(), template);
    }

    /// Get a template by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CodeTemplate> {
        self.templates.get(name)
    }

    /// List all template names.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.templates.keys().map(String::as_str).collect()
    }

    /// List templates for a specific language.
    #[must_use]
    pub fn for_language(&self, lang: Language) -> Vec<&CodeTemplate> {
        self.templates
            .values()
            .filter(|t| t.language == lang)
            .collect()
    }

    /// Number of templates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Generate code from a named template.
    #[must_use]
    pub fn generate_code(
        &self,
        template_name: &str,
        vars: &HashMap<String, String>,
    ) -> Option<String> {
        self.get(template_name).map(|t| t.render(vars))
    }

    fn register_builtins(&mut self) {
        self.register(CodeTemplate::new(
            "rust_struct",
            Language::Rust,
            "/// {{description}}\n#[derive(Debug, Clone)]\npub struct {{name}} {\n    {{fields}}\n}",
            vec!["name".into(), "description".into(), "fields".into()],
        ));
        self.register(CodeTemplate::new(
            "rust_impl",
            Language::Rust,
            "impl {{name}} {\n    #[must_use]\n    pub fn new({{params}}) -> Self {\n        Self { {{init}} }\n    }\n}",
            vec!["name".into(), "params".into(), "init".into()],
        ));
        self.register(CodeTemplate::new(
            "rust_test",
            Language::Rust,
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn {{test_name}}() {\n        {{body}}\n    }\n}",
            vec!["test_name".into(), "body".into()],
        ));
        self.register(CodeTemplate::new(
            "rust_module",
            Language::Rust,
            "//! {{description}}\n\n{{body}}",
            vec!["description".into(), "body".into()],
        ));
        self.register(CodeTemplate::new(
            "typescript_component",
            Language::TypeScript,
            "import React from 'react';\n\ninterface {{name}}Props {\n    {{props}}\n}\n\nexport const {{name}}: React.FC<{{name}}Props> = ({ {{destructured}} }) => {\n    return (\n        {{jsx}}\n    );\n};",
            vec!["name".into(), "props".into(), "destructured".into(), "jsx".into()],
        ));
        self.register(CodeTemplate::new(
            "python_class",
            Language::Python,
            "class {{name}}:\n    \"\"\"{{description}}\"\"\"\n\n    def __init__(self, {{params}}):\n        {{init}}",
            vec!["name".into(), "description".into(), "params".into(), "init".into()],
        ));
        self.register(CodeTemplate::new(
            "go_struct",
            Language::Go,
            "// {{name}} {{description}}\ntype {{name}} struct {\n    {{fields}}\n}",
            vec!["name".into(), "description".into(), "fields".into()],
        ));
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_rust() {
        assert_eq!(detect_language("src/main.rs"), Language::Rust);
    }

    #[test]
    fn detect_typescript() {
        assert_eq!(detect_language("app.tsx"), Language::TypeScript);
        assert_eq!(detect_language("index.ts"), Language::TypeScript);
    }

    #[test]
    fn detect_javascript() {
        assert_eq!(detect_language("app.js"), Language::JavaScript);
        assert_eq!(detect_language("util.mjs"), Language::JavaScript);
    }

    #[test]
    fn detect_python() {
        assert_eq!(detect_language("main.py"), Language::Python);
    }

    #[test]
    fn detect_go() {
        assert_eq!(detect_language("main.go"), Language::Go);
    }

    #[test]
    fn detect_unknown() {
        assert_eq!(detect_language("file.xyz"), Language::Unknown);
        assert_eq!(detect_language("noext"), Language::Unknown);
    }

    #[test]
    fn language_display() {
        assert_eq!(Language::Rust.to_string(), "rust");
        assert_eq!(Language::TypeScript.to_string(), "typescript");
    }

    #[test]
    fn template_render() {
        let tmpl = CodeTemplate::new(
            "test",
            Language::Rust,
            "struct {{name}} { {{fields}} }",
            vec!["name".into(), "fields".into()],
        );
        let mut vars = HashMap::new();
        vars.insert("name".into(), "Foo".into());
        vars.insert("fields".into(), "x: i32".into());
        assert_eq!(tmpl.render(&vars), "struct Foo { x: i32 }");
    }

    #[test]
    fn template_render_missing_var() {
        let tmpl = CodeTemplate::new(
            "test",
            Language::Rust,
            "hello {{name}}",
            vec!["name".into()],
        );
        let vars = HashMap::new();
        assert_eq!(tmpl.render(&vars), "hello {{name}}");
    }

    #[test]
    fn library_empty() {
        let lib = TemplateLibrary::new();
        assert!(lib.is_empty());
        assert_eq!(lib.len(), 0);
    }

    #[test]
    fn library_with_builtins() {
        let lib = TemplateLibrary::with_builtins();
        assert!(lib.len() >= 7);
        assert!(lib.get("rust_struct").is_some());
        assert!(lib.get("rust_impl").is_some());
        assert!(lib.get("rust_test").is_some());
        assert!(lib.get("rust_module").is_some());
        assert!(lib.get("typescript_component").is_some());
        assert!(lib.get("python_class").is_some());
        assert!(lib.get("go_struct").is_some());
    }

    #[test]
    fn library_register_custom() {
        let mut lib = TemplateLibrary::new();
        lib.register(CodeTemplate::new("custom", Language::Rust, "hi", vec![]));
        assert_eq!(lib.len(), 1);
        assert!(lib.get("custom").is_some());
    }

    #[test]
    fn library_for_language() {
        let lib = TemplateLibrary::with_builtins();
        let rust_templates = lib.for_language(Language::Rust);
        assert!(rust_templates.len() >= 4);
        for t in &rust_templates {
            assert_eq!(t.language, Language::Rust);
        }
    }

    #[test]
    fn library_names() {
        let lib = TemplateLibrary::with_builtins();
        let names = lib.names();
        assert!(names.contains(&"rust_struct"));
    }

    #[test]
    fn generate_rust_struct() {
        let lib = TemplateLibrary::with_builtins();
        let mut vars = HashMap::new();
        vars.insert("name".into(), "Config".into());
        vars.insert("description".into(), "App configuration".into());
        vars.insert("fields".into(), "pub port: u16,".into());
        let code = lib.generate_code("rust_struct", &vars).unwrap();
        assert!(code.contains("pub struct Config"));
        assert!(code.contains("App configuration"));
        assert!(code.contains("pub port: u16,"));
    }

    #[test]
    fn generate_rust_test() {
        let lib = TemplateLibrary::with_builtins();
        let mut vars = HashMap::new();
        vars.insert("test_name".into(), "it_works".into());
        vars.insert("body".into(), "assert!(true);".into());
        let code = lib.generate_code("rust_test", &vars).unwrap();
        assert!(code.contains("#[cfg(test)]"));
        assert!(code.contains("fn it_works()"));
    }

    #[test]
    fn generate_typescript_component() {
        let lib = TemplateLibrary::with_builtins();
        let mut vars = HashMap::new();
        vars.insert("name".into(), "Button".into());
        vars.insert("props".into(), "label: string;".into());
        vars.insert("destructured".into(), "label".into());
        vars.insert("jsx".into(), "<button>{label}</button>".into());
        let code = lib.generate_code("typescript_component", &vars).unwrap();
        assert!(code.contains("ButtonProps"));
        assert!(code.contains("React.FC"));
    }

    #[test]
    fn generate_nonexistent() {
        let lib = TemplateLibrary::with_builtins();
        assert!(lib.generate_code("nope", &HashMap::new()).is_none());
    }

    #[test]
    fn generate_python_class() {
        let lib = TemplateLibrary::with_builtins();
        let mut vars = HashMap::new();
        vars.insert("name".into(), "Parser".into());
        vars.insert("description".into(), "A simple parser".into());
        vars.insert("params".into(), "text: str".into());
        vars.insert("init".into(), "self.text = text".into());
        let code = lib.generate_code("python_class", &vars).unwrap();
        assert!(code.contains("class Parser:"));
        assert!(code.contains("self.text = text"));
    }
}
