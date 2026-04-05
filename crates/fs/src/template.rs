//! Project template system and scaffolding generation.
//!
//! Provides the foundation for `crab init <template>`: template manifests
//! (TOML or JSON), variable substitution (`{{var_name}}`), template discovery
//! from `~/.crab/templates/`, and scaffold generation into a target directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Template manifest describing a project template.
///
/// Located at `template.toml` or `template.json` inside a template directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateManifest {
    /// Human-readable template name (e.g. "rust-cli").
    pub name: String,
    /// Short description of what the template creates.
    #[serde(default)]
    pub description: String,
    /// Template version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Author or maintainer.
    #[serde(default)]
    pub author: String,
    /// Variable definitions with optional defaults.
    #[serde(default)]
    pub variables: Vec<VariableDef>,
    /// Files to exclude from the template output (glob patterns).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Post-generation commands to run (informational only at this layer).
    #[serde(default)]
    pub post_create: Vec<String>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// Definition of a template variable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableDef {
    /// Variable name (used as `{{name}}` in templates).
    pub name: String,
    /// Human-readable prompt for the variable.
    #[serde(default)]
    pub prompt: String,
    /// Default value if the user does not supply one.
    pub default: Option<String>,
    /// Whether this variable is required.
    #[serde(default)]
    pub required: bool,
}

/// A discovered template ready to be instantiated.
#[derive(Debug, Clone)]
pub struct Template {
    /// Parsed manifest.
    pub manifest: TemplateManifest,
    /// Root directory of the template on disk.
    pub root: PathBuf,
}

/// Result of scaffolding a template into a target directory.
#[derive(Debug, Clone)]
pub struct ScaffoldResult {
    /// Target directory where files were written.
    pub target_dir: PathBuf,
    /// List of files created (relative to `target_dir`).
    pub files_created: Vec<PathBuf>,
    /// Post-create commands from the manifest (for CLI to display/run).
    pub post_create: Vec<String>,
}

// ---------------------------------------------------------------------------
// Manifest parsing
// ---------------------------------------------------------------------------

/// Parse a [`TemplateManifest`] from a TOML string.
///
/// # Errors
///
/// Returns an error if the TOML is invalid or missing required fields.
pub fn parse_manifest_toml(content: &str) -> crab_common::Result<TemplateManifest> {
    toml::from_str(content)
        .map_err(|e| crab_common::Error::Other(format!("invalid template manifest TOML: {e}")))
}

/// Parse a [`TemplateManifest`] from a JSON string.
///
/// # Errors
///
/// Returns an error if the JSON is invalid or missing required fields.
pub fn parse_manifest_json(content: &str) -> crab_common::Result<TemplateManifest> {
    serde_json::from_str(content)
        .map_err(|e| crab_common::Error::Other(format!("invalid template manifest JSON: {e}")))
}

/// Load a [`TemplateManifest`] from a template directory.
///
/// Looks for `template.toml` first, then `template.json`.
///
/// # Errors
///
/// Returns an error if neither manifest file exists or cannot be parsed.
pub fn load_manifest(template_dir: &Path) -> crab_common::Result<TemplateManifest> {
    let toml_path = template_dir.join("template.toml");
    if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path).map_err(|e| {
            crab_common::Error::Other(format!("cannot read {}: {e}", toml_path.display()))
        })?;
        return parse_manifest_toml(&content);
    }

    let json_path = template_dir.join("template.json");
    if json_path.exists() {
        let content = std::fs::read_to_string(&json_path).map_err(|e| {
            crab_common::Error::Other(format!("cannot read {}: {e}", json_path.display()))
        })?;
        return parse_manifest_json(&content);
    }

    Err(crab_common::Error::Other(format!(
        "no template.toml or template.json found in {}",
        template_dir.display()
    )))
}

// ---------------------------------------------------------------------------
// Variable substitution
// ---------------------------------------------------------------------------

/// Apply variable substitution to a string.
///
/// Replaces all occurrences of `{{key}}` with the corresponding value.
/// Whitespace inside braces is trimmed: `{{ key }}` also works.
///
/// # Errors
///
/// Returns an error if a `{{var}}` placeholder has no matching value in `vars`
/// and no default exists.
#[allow(clippy::implicit_hasher)]
pub fn substitute(content: &str, vars: &HashMap<String, String>) -> crab_common::Result<String> {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;

    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];

        let end = after_open
            .find("}}")
            .ok_or_else(|| crab_common::Error::Other("unclosed {{ in template".into()))?;

        let var_name = after_open[..end].trim();

        let value = vars.get(var_name).ok_or_else(|| {
            crab_common::Error::Other(format!("template variable '{var_name}' not provided"))
        })?;

        result.push_str(value);
        rest = &after_open[end + 2..];
    }

    result.push_str(rest);
    Ok(result)
}

// ---------------------------------------------------------------------------
// Template discovery
// ---------------------------------------------------------------------------

/// Discover all templates in a directory (typically `~/.crab/templates/`).
///
/// Each subdirectory containing a `template.toml` or `template.json` is
/// treated as a template.
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn discover_templates(templates_dir: &Path) -> crab_common::Result<Vec<Template>> {
    if !templates_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(templates_dir).map_err(|e| {
        crab_common::Error::Other(format!(
            "cannot read templates directory {}: {e}",
            templates_dir.display()
        ))
    })?;

    let mut templates = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| {
            crab_common::Error::Other(format!("error reading directory entry: {e}"))
        })?;

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Try to load the manifest — skip directories without one
        if let Ok(manifest) = load_manifest(&path) {
            templates.push(Template {
                manifest,
                root: path,
            });
        }
    }

    templates.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    Ok(templates)
}

/// Find a template by name in a templates directory.
///
/// # Errors
///
/// Returns an error if the template is not found.
pub fn find_template(templates_dir: &Path, name: &str) -> crab_common::Result<Template> {
    let templates = discover_templates(templates_dir)?;
    templates
        .into_iter()
        .find(|t| t.manifest.name == name)
        .ok_or_else(|| {
            crab_common::Error::Other(format!(
                "template '{name}' not found in {}",
                templates_dir.display()
            ))
        })
}

// ---------------------------------------------------------------------------
// Scaffold generation
// ---------------------------------------------------------------------------

/// Resolve variables: apply defaults from manifest, then check all required
/// variables are present.
///
/// # Errors
///
/// Returns an error if a required variable is missing.
#[allow(clippy::implicit_hasher)]
pub fn resolve_variables(
    manifest: &TemplateManifest,
    user_vars: &HashMap<String, String>,
) -> crab_common::Result<HashMap<String, String>> {
    let mut resolved = user_vars.clone();

    for var_def in &manifest.variables {
        if !resolved.contains_key(&var_def.name) {
            if let Some(default) = &var_def.default {
                resolved.insert(var_def.name.clone(), default.clone());
            } else if var_def.required {
                return Err(crab_common::Error::Other(format!(
                    "required template variable '{}' not provided",
                    var_def.name
                )));
            }
        }
    }

    Ok(resolved)
}

/// Generate a project from a template into `target_dir`.
///
/// Copies all files from the template (excluding the manifest and files
/// matching `exclude` patterns), applying variable substitution to text
/// files. Binary files are copied as-is.
///
/// # Errors
///
/// Returns an error if the target directory already contains files, if
/// variable substitution fails, or if I/O operations fail.
#[allow(clippy::implicit_hasher)]
pub fn scaffold(
    template: &Template,
    target_dir: &Path,
    vars: &HashMap<String, String>,
) -> crab_common::Result<ScaffoldResult> {
    let resolved = resolve_variables(&template.manifest, vars)?;

    // Create target directory
    std::fs::create_dir_all(target_dir).map_err(|e| {
        crab_common::Error::Other(format!(
            "cannot create target directory {}: {e}",
            target_dir.display()
        ))
    })?;

    let mut files_created = Vec::new();
    let exclude_set = build_exclude_set(&template.manifest.exclude);

    copy_template_dir(
        &template.root,
        &template.root,
        target_dir,
        &resolved,
        &exclude_set,
        &mut files_created,
    )?;

    Ok(ScaffoldResult {
        target_dir: target_dir.to_path_buf(),
        files_created,
        post_create: template.manifest.post_create.clone(),
    })
}

/// Build a set of exclude patterns from glob strings.
fn build_exclude_set(patterns: &[String]) -> Vec<globset::GlobMatcher> {
    patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect()
}

/// Recursively copy a template directory, applying substitution.
fn copy_template_dir(
    template_root: &Path,
    current_dir: &Path,
    target_base: &Path,
    vars: &HashMap<String, String>,
    exclude: &[globset::GlobMatcher],
    files_created: &mut Vec<PathBuf>,
) -> crab_common::Result<()> {
    let entries = std::fs::read_dir(current_dir).map_err(|e| {
        crab_common::Error::Other(format!(
            "cannot read directory {}: {e}",
            current_dir.display()
        ))
    })?;

    for entry in entries {
        let entry =
            entry.map_err(|e| crab_common::Error::Other(format!("error reading entry: {e}")))?;

        let source_path = entry.path();
        let rel_path = source_path
            .strip_prefix(template_root)
            .unwrap_or(&source_path);

        // Skip manifest files
        let file_name = source_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        if file_name == "template.toml" || file_name == "template.json" {
            continue;
        }

        // Check exclude patterns
        let rel_str = rel_path.to_string_lossy();
        if exclude.iter().any(|g| g.is_match(rel_str.as_ref())) {
            continue;
        }

        // Substitute variables in the file/directory name itself
        let target_name = substitute(&file_name, vars).unwrap_or_else(|_| file_name.to_string());
        let target_path = target_base
            .join(rel_path.parent().unwrap_or_else(|| Path::new("")))
            .join(&target_name);

        if source_path.is_dir() {
            std::fs::create_dir_all(&target_path).map_err(|e| {
                crab_common::Error::Other(format!(
                    "cannot create directory {}: {e}",
                    target_path.display()
                ))
            })?;
            copy_template_dir(
                template_root,
                &source_path,
                target_base,
                vars,
                exclude,
                files_created,
            )?;
        } else {
            // Ensure parent exists
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    crab_common::Error::Other(format!(
                        "cannot create directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }

            let content = std::fs::read(&source_path).map_err(|e| {
                crab_common::Error::Other(format!("cannot read {}: {e}", source_path.display()))
            })?;

            // Only apply substitution to text files
            if is_text_content(&content) {
                let text = String::from_utf8_lossy(&content);
                let substituted = substitute(&text, vars)?;
                std::fs::write(&target_path, substituted.as_bytes()).map_err(|e| {
                    crab_common::Error::Other(format!(
                        "cannot write {}: {e}",
                        target_path.display()
                    ))
                })?;
            } else {
                std::fs::write(&target_path, &content).map_err(|e| {
                    crab_common::Error::Other(format!(
                        "cannot write {}: {e}",
                        target_path.display()
                    ))
                })?;
            }

            let created_rel = target_path
                .strip_prefix(target_base)
                .unwrap_or(&target_path)
                .to_path_buf();
            files_created.push(created_rel);
        }
    }

    Ok(())
}

/// Simple text detection: no null bytes in the first 8KB.
fn is_text_content(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    !data[..check_len].contains(&0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Manifest parsing ─────────────────────────────────────────

    #[test]
    fn parse_toml_manifest_minimal() {
        let toml = r#"
name = "rust-cli"
"#;
        let m = parse_manifest_toml(toml).unwrap();
        assert_eq!(m.name, "rust-cli");
        assert_eq!(m.version, "0.1.0");
        assert!(m.variables.is_empty());
    }

    #[test]
    fn parse_toml_manifest_full() {
        let toml = r#"
name = "rust-cli"
description = "A CLI project template"
version = "1.0.0"
author = "test"
exclude = ["*.bak", ".git"]
post_create = ["cargo build"]

[[variables]]
name = "project_name"
prompt = "Project name?"
required = true

[[variables]]
name = "license"
prompt = "License?"
default = "MIT"
"#;
        let m = parse_manifest_toml(toml).unwrap();
        assert_eq!(m.name, "rust-cli");
        assert_eq!(m.description, "A CLI project template");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.variables.len(), 2);
        assert!(m.variables[0].required);
        assert_eq!(m.variables[1].default.as_deref(), Some("MIT"));
        assert_eq!(m.exclude.len(), 2);
        assert_eq!(m.post_create, vec!["cargo build"]);
    }

    #[test]
    fn parse_json_manifest() {
        let json = r#"{
            "name": "node-app",
            "description": "Node.js app",
            "variables": [
                { "name": "project_name", "prompt": "Name?", "required": true }
            ]
        }"#;
        let m = parse_manifest_json(json).unwrap();
        assert_eq!(m.name, "node-app");
        assert_eq!(m.variables.len(), 1);
    }

    #[test]
    fn parse_invalid_toml_errors() {
        let result = parse_manifest_toml("not valid { toml");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid template"));
    }

    #[test]
    fn parse_invalid_json_errors() {
        let result = parse_manifest_json("{bad json");
        assert!(result.is_err());
    }

    // ── Variable substitution ────────────────────────────────────

    #[test]
    fn substitute_basic() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "my-project".to_string());
        let result = substitute("Hello {{name}}!", &vars).unwrap();
        assert_eq!(result, "Hello my-project!");
    }

    #[test]
    fn substitute_with_spaces() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "test".to_string());
        let result = substitute("Hello {{ name }}!", &vars).unwrap();
        assert_eq!(result, "Hello test!");
    }

    #[test]
    fn substitute_multiple() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), "X".to_string());
        vars.insert("b".to_string(), "Y".to_string());
        let result = substitute("{{a}} and {{b}}", &vars).unwrap();
        assert_eq!(result, "X and Y");
    }

    #[test]
    fn substitute_no_placeholders() {
        let vars = HashMap::new();
        let result = substitute("no vars here", &vars).unwrap();
        assert_eq!(result, "no vars here");
    }

    #[test]
    fn substitute_missing_variable_errors() {
        let vars = HashMap::new();
        let result = substitute("{{missing}}", &vars);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }

    #[test]
    fn substitute_unclosed_brace_errors() {
        let vars = HashMap::new();
        let result = substitute("{{unclosed", &vars);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unclosed"));
    }

    #[test]
    fn substitute_empty_value() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), String::new());
        let result = substitute("pre-{{name}}-post", &vars).unwrap();
        assert_eq!(result, "pre--post");
    }

    #[test]
    fn substitute_repeated_variable() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), "val".to_string());
        let result = substitute("{{x}}/{{x}}/{{x}}", &vars).unwrap();
        assert_eq!(result, "val/val/val");
    }

    // ── Manifest loading from filesystem ─────────────────────────

    #[test]
    fn load_manifest_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("template.toml"),
            "name = \"test-template\"\n",
        )
        .unwrap();
        let m = load_manifest(dir.path()).unwrap();
        assert_eq!(m.name, "test-template");
    }

    #[test]
    fn load_manifest_json_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("template.json"),
            r#"{"name": "json-template"}"#,
        )
        .unwrap();
        let m = load_manifest(dir.path()).unwrap();
        assert_eq!(m.name, "json-template");
    }

    #[test]
    fn load_manifest_toml_preferred_over_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("template.toml"), "name = \"from-toml\"\n").unwrap();
        fs::write(dir.path().join("template.json"), r#"{"name": "from-json"}"#).unwrap();
        let m = load_manifest(dir.path()).unwrap();
        assert_eq!(m.name, "from-toml");
    }

    #[test]
    fn load_manifest_no_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_manifest(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no template.toml"));
    }

    // ── Template discovery ───────────────────────────────────────

    #[test]
    fn discover_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let templates = discover_templates(dir.path()).unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn discover_nonexistent_dir() {
        let templates = discover_templates(Path::new("/nonexistent/templates")).unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn discover_multiple_templates() {
        let dir = tempfile::tempdir().unwrap();

        // Template A
        let a_dir = dir.path().join("alpha");
        fs::create_dir(&a_dir).unwrap();
        fs::write(a_dir.join("template.toml"), "name = \"alpha\"\n").unwrap();

        // Template B
        let b_dir = dir.path().join("beta");
        fs::create_dir(&b_dir).unwrap();
        fs::write(b_dir.join("template.toml"), "name = \"beta\"\n").unwrap();

        // Non-template file (should be skipped)
        fs::write(dir.path().join("random.txt"), "ignore").unwrap();

        let templates = discover_templates(dir.path()).unwrap();
        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].manifest.name, "alpha");
        assert_eq!(templates[1].manifest.name, "beta");
    }

    #[test]
    fn find_template_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let t_dir = dir.path().join("my-tmpl");
        fs::create_dir(&t_dir).unwrap();
        fs::write(t_dir.join("template.toml"), "name = \"my-tmpl\"\n").unwrap();

        let template = find_template(dir.path(), "my-tmpl").unwrap();
        assert_eq!(template.manifest.name, "my-tmpl");
    }

    #[test]
    fn find_template_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_template(dir.path(), "nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // ── Variable resolution ──────────────────────────────────────

    #[test]
    fn resolve_applies_defaults() {
        let manifest = TemplateManifest {
            name: "test".to_string(),
            description: String::new(),
            version: "0.1.0".to_string(),
            author: String::new(),
            variables: vec![VariableDef {
                name: "license".to_string(),
                prompt: String::new(),
                default: Some("MIT".to_string()),
                required: false,
            }],
            exclude: vec![],
            post_create: vec![],
        };

        let vars = HashMap::new();
        let resolved = resolve_variables(&manifest, &vars).unwrap();
        assert_eq!(resolved.get("license").unwrap(), "MIT");
    }

    #[test]
    fn resolve_user_overrides_default() {
        let manifest = TemplateManifest {
            name: "test".to_string(),
            description: String::new(),
            version: "0.1.0".to_string(),
            author: String::new(),
            variables: vec![VariableDef {
                name: "license".to_string(),
                prompt: String::new(),
                default: Some("MIT".to_string()),
                required: false,
            }],
            exclude: vec![],
            post_create: vec![],
        };

        let mut vars = HashMap::new();
        vars.insert("license".to_string(), "Apache-2.0".to_string());
        let resolved = resolve_variables(&manifest, &vars).unwrap();
        assert_eq!(resolved.get("license").unwrap(), "Apache-2.0");
    }

    #[test]
    fn resolve_required_missing_errors() {
        let manifest = TemplateManifest {
            name: "test".to_string(),
            description: String::new(),
            version: "0.1.0".to_string(),
            author: String::new(),
            variables: vec![VariableDef {
                name: "project_name".to_string(),
                prompt: String::new(),
                default: None,
                required: true,
            }],
            exclude: vec![],
            post_create: vec![],
        };

        let vars = HashMap::new();
        let result = resolve_variables(&manifest, &vars);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("project_name"));
    }

    // ── Scaffolding ──────────────────────────────────────────────

    #[test]
    fn scaffold_basic_template() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        // Create template
        fs::write(
            tmpl_dir.path().join("template.toml"),
            r#"
name = "basic"
[[variables]]
name = "project_name"
required = true
"#,
        )
        .unwrap();
        fs::write(
            tmpl_dir.path().join("README.md"),
            "# {{project_name}}\n\nWelcome to {{project_name}}.\n",
        )
        .unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let mut vars = HashMap::new();
        vars.insert("project_name".to_string(), "my-app".to_string());

        let result = scaffold(&template, target_dir.path(), &vars).unwrap();

        assert_eq!(result.files_created.len(), 1);
        let readme = fs::read_to_string(target_dir.path().join("README.md")).unwrap();
        assert_eq!(readme, "# my-app\n\nWelcome to my-app.\n");
    }

    #[test]
    fn scaffold_nested_directories() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        fs::write(tmpl_dir.path().join("template.toml"), "name = \"nested\"\n").unwrap();

        let src_dir = tmpl_dir.path().join("src");
        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("main.rs"), "fn main() {}\n").unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let vars = HashMap::new();
        let result = scaffold(&template, target_dir.path(), &vars).unwrap();

        assert!(result.files_created.iter().any(|f| f.ends_with("main.rs")));
        assert!(target_dir.path().join("src").join("main.rs").exists());
    }

    #[test]
    fn scaffold_excludes_patterns() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        fs::write(
            tmpl_dir.path().join("template.toml"),
            "name = \"exclude-test\"\nexclude = [\"*.bak\"]\n",
        )
        .unwrap();
        fs::write(tmpl_dir.path().join("good.txt"), "keep").unwrap();
        fs::write(tmpl_dir.path().join("old.bak"), "discard").unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let vars = HashMap::new();
        let result = scaffold(&template, target_dir.path(), &vars).unwrap();

        assert!(target_dir.path().join("good.txt").exists());
        assert!(!target_dir.path().join("old.bak").exists());
        assert_eq!(result.files_created.len(), 1);
    }

    #[test]
    fn scaffold_skips_manifest_files() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        fs::write(
            tmpl_dir.path().join("template.toml"),
            "name = \"no-manifest\"\n",
        )
        .unwrap();
        fs::write(tmpl_dir.path().join("file.txt"), "content").unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let vars = HashMap::new();
        let result = scaffold(&template, target_dir.path(), &vars).unwrap();

        // template.toml should NOT be copied
        assert!(!target_dir.path().join("template.toml").exists());
        assert!(target_dir.path().join("file.txt").exists());
        assert_eq!(result.files_created.len(), 1);
    }

    #[test]
    fn scaffold_binary_file_copied_as_is() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        fs::write(tmpl_dir.path().join("template.toml"), "name = \"binary\"\n").unwrap();

        // Binary content with null bytes
        let binary_data: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0x01, 0x02];
        fs::write(tmpl_dir.path().join("image.png"), &binary_data).unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let vars = HashMap::new();
        scaffold(&template, target_dir.path(), &vars).unwrap();

        let copied = fs::read(target_dir.path().join("image.png")).unwrap();
        assert_eq!(copied, binary_data);
    }

    #[test]
    fn scaffold_post_create_passed_through() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        fs::write(
            tmpl_dir.path().join("template.toml"),
            "name = \"post\"\npost_create = [\"cargo build\", \"cargo test\"]\n",
        )
        .unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let result = scaffold(&template, target_dir.path(), &HashMap::new()).unwrap();
        assert_eq!(result.post_create, vec!["cargo build", "cargo test"]);
    }

    #[test]
    fn scaffold_missing_required_var_errors() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        fs::write(
            tmpl_dir.path().join("template.toml"),
            "name = \"req\"\n\n[[variables]]\nname = \"x\"\nrequired = true\n",
        )
        .unwrap();

        let template = Template {
            manifest: load_manifest(tmpl_dir.path()).unwrap(),
            root: tmpl_dir.path().to_path_buf(),
        };

        let result = scaffold(&template, target_dir.path(), &HashMap::new());
        assert!(result.is_err());
    }

    // ── Text detection ───────────────────────────────────────────

    #[test]
    fn is_text_content_utf8() {
        assert!(is_text_content(b"hello world"));
    }

    #[test]
    fn is_text_content_empty() {
        assert!(is_text_content(b""));
    }

    #[test]
    fn is_text_content_binary() {
        assert!(!is_text_content(&[0xFF, 0x00, 0x01]));
    }

    // ── Manifest serialization round-trip ────────────────────────

    #[test]
    fn manifest_toml_round_trip() {
        let manifest = TemplateManifest {
            name: "roundtrip".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: "dev".to_string(),
            variables: vec![VariableDef {
                name: "x".to_string(),
                prompt: "X?".to_string(),
                default: Some("y".to_string()),
                required: false,
            }],
            exclude: vec!["*.tmp".to_string()],
            post_create: vec!["echo done".to_string()],
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        let parsed = parse_manifest_toml(&toml_str).unwrap();
        assert_eq!(parsed.name, "roundtrip");
        assert_eq!(parsed.variables.len(), 1);
    }

    #[test]
    fn manifest_json_round_trip() {
        let manifest = TemplateManifest {
            name: "json-rt".to_string(),
            description: String::new(),
            version: "0.1.0".to_string(),
            author: String::new(),
            variables: vec![],
            exclude: vec![],
            post_create: vec![],
        };

        let json_str = serde_json::to_string(&manifest).unwrap();
        let parsed = parse_manifest_json(&json_str).unwrap();
        assert_eq!(parsed.name, "json-rt");
    }
}
