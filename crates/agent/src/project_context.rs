//! Project analysis: type detection, dependency graph, file importance, and summary generation.
//!
//! Scans a project directory to build contextual understanding that can be
//! injected into the system prompt for more informed agent behavior.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

// ── Project type detection ──────────────────────────────────────────

/// Recognized project types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Php,
    Swift,
    Kotlin,
    Unknown,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "Rust"),
            Self::Node => write!(f, "Node.js"),
            Self::Python => write!(f, "Python"),
            Self::Go => write!(f, "Go"),
            Self::Java => write!(f, "Java"),
            Self::CSharp => write!(f, "C#"),
            Self::Ruby => write!(f, "Ruby"),
            Self::Php => write!(f, "PHP"),
            Self::Swift => write!(f, "Swift"),
            Self::Kotlin => write!(f, "Kotlin"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Marker file that signals a project type.
struct ProjectMarker {
    filename: &'static str,
    project_type: ProjectType,
}

const PROJECT_MARKERS: &[ProjectMarker] = &[
    ProjectMarker {
        filename: "Cargo.toml",
        project_type: ProjectType::Rust,
    },
    ProjectMarker {
        filename: "package.json",
        project_type: ProjectType::Node,
    },
    ProjectMarker {
        filename: "pyproject.toml",
        project_type: ProjectType::Python,
    },
    ProjectMarker {
        filename: "setup.py",
        project_type: ProjectType::Python,
    },
    ProjectMarker {
        filename: "requirements.txt",
        project_type: ProjectType::Python,
    },
    ProjectMarker {
        filename: "go.mod",
        project_type: ProjectType::Go,
    },
    ProjectMarker {
        filename: "pom.xml",
        project_type: ProjectType::Java,
    },
    ProjectMarker {
        filename: "build.gradle",
        project_type: ProjectType::Java,
    },
    ProjectMarker {
        filename: "build.gradle.kts",
        project_type: ProjectType::Kotlin,
    },
    ProjectMarker {
        filename: "*.csproj",
        project_type: ProjectType::CSharp,
    },
    ProjectMarker {
        filename: "*.sln",
        project_type: ProjectType::CSharp,
    },
    ProjectMarker {
        filename: "Gemfile",
        project_type: ProjectType::Ruby,
    },
    ProjectMarker {
        filename: "composer.json",
        project_type: ProjectType::Php,
    },
    ProjectMarker {
        filename: "Package.swift",
        project_type: ProjectType::Swift,
    },
];

/// Detect the project type(s) by checking for marker files.
#[must_use]
pub fn detect_project_type(project_dir: &Path) -> Vec<ProjectType> {
    let mut types = Vec::new();

    for marker in PROJECT_MARKERS {
        if marker.filename.starts_with('*') {
            // Glob-style: check for any file with that extension
            let ext = &marker.filename[1..]; // e.g. ".csproj"
            if let Ok(entries) = std::fs::read_dir(project_dir) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().ends_with(ext)
                        && !types.contains(&marker.project_type)
                    {
                        types.push(marker.project_type);
                        break;
                    }
                }
            }
        } else if project_dir.join(marker.filename).exists()
            && !types.contains(&marker.project_type)
        {
            types.push(marker.project_type);
        }
    }

    if types.is_empty() {
        types.push(ProjectType::Unknown);
    }

    types
}

// ── Dependency graph ────────────────────────────────────────────────

/// A dependency relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The dependent package/module name.
    pub name: String,
    /// Optional version constraint.
    pub version: Option<String>,
    /// Whether this is a dev/test dependency.
    pub dev: bool,
}

/// A simple dependency graph: package -> dependencies.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Maps package names to their dependencies.
    pub packages: HashMap<String, Vec<Dependency>>,
}

impl DependencyGraph {
    /// Create a new empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a package with its dependencies.
    pub fn add_package(&mut self, name: impl Into<String>, deps: Vec<Dependency>) {
        self.packages.insert(name.into(), deps);
    }

    /// Get dependencies for a package.
    #[must_use]
    pub fn deps_for(&self, package: &str) -> Option<&[Dependency]> {
        self.packages.get(package).map(Vec::as_slice)
    }

    /// Total number of packages.
    #[must_use]
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Total number of dependencies across all packages.
    #[must_use]
    pub fn total_deps(&self) -> usize {
        self.packages.values().map(Vec::len).sum()
    }

    /// Find packages that depend on the given package.
    #[must_use]
    pub fn reverse_deps(&self, target: &str) -> Vec<&str> {
        self.packages
            .iter()
            .filter(|(_, deps)| deps.iter().any(|d| d.name == target))
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

/// Parse a basic dependency graph from a Cargo workspace.
///
/// This is a lightweight parser that extracts `[dependencies]` from `Cargo.toml`.
/// It does not invoke `cargo metadata` (too slow for prompt context).
#[must_use]
pub fn parse_cargo_deps(cargo_toml_content: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    let mut in_dev_deps = false;

    for line in cargo_toml_content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]";
            in_dev_deps = trimmed == "[dev-dependencies]";
            continue;
        }

        if (!in_deps && !in_dev_deps) || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse "name = ..." or "name.workspace = true"
        if let Some((name, rest)) = trimmed.split_once('=') {
            let name = name.trim().to_string();
            if name.contains('.') {
                // e.g. "serde.workspace = true" — extract the base name
                if let Some((base, _)) = name.split_once('.') {
                    deps.push(Dependency {
                        name: base.trim().to_string(),
                        version: None,
                        dev: in_dev_deps,
                    });
                }
            } else {
                let rest = rest.trim().trim_matches('"');
                let version = if rest.is_empty() || rest.starts_with('{') {
                    None
                } else {
                    Some(rest.to_string())
                };
                deps.push(Dependency {
                    name,
                    version,
                    dev: in_dev_deps,
                });
            }
        }
    }

    deps
}

// ── File importance scoring ─────────────────────────────────────────

/// Score for a single file, indicating its relative importance.
#[derive(Debug, Clone)]
pub struct FileScore {
    pub path: PathBuf,
    /// Normalized score from 0.0 to 1.0.
    pub score: f64,
    /// Breakdown of scoring factors.
    pub factors: Vec<(String, f64)>,
}

/// Score files by importance within a project.
///
/// Factors considered:
/// - Entry points (main.rs, lib.rs, index.js, etc.) get a boost
/// - Configuration files (Cargo.toml, package.json) get a boost
/// - Test files get a small reduction
/// - Depth in directory tree (shallower = more important)
#[must_use]
pub fn score_files(files: &[PathBuf], project_type: ProjectType) -> Vec<FileScore> {
    if files.is_empty() {
        return Vec::new();
    }

    #[allow(clippy::cast_precision_loss)] // path depth is always small
    let max_depth = files
        .iter()
        .map(|p| p.components().count())
        .max()
        .unwrap_or(1) as f64;

    let mut scores: Vec<FileScore> = files
        .iter()
        .map(|path| {
            let mut factors = Vec::new();
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Entry point bonus
            let entry_bonus = entry_point_score(&filename, project_type);
            if entry_bonus > 0.0 {
                factors.push(("entry_point".into(), entry_bonus));
            }

            // Config file bonus
            let config_bonus = config_file_score(&filename);
            if config_bonus > 0.0 {
                factors.push(("config_file".into(), config_bonus));
            }

            // Test file reduction
            let test_penalty = test_file_penalty(&filename, path);
            if test_penalty < 0.0 {
                factors.push(("test_file".into(), test_penalty));
            }

            // Depth factor: shallower files are more important
            #[allow(clippy::cast_precision_loss)]
            let depth = path.components().count() as f64;
            let depth_score = (depth / max_depth).mul_add(-0.5, 1.0);
            factors.push(("depth".into(), depth_score));

            let raw_score: f64 = factors.iter().map(|(_, s)| s).sum();
            FileScore {
                path: path.clone(),
                score: raw_score,
                factors,
            }
        })
        .collect();

    // Normalize to 0.0-1.0
    let min_score = scores.iter().map(|s| s.score).fold(f64::INFINITY, f64::min);
    let max_score = scores
        .iter()
        .map(|s| s.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = max_score - min_score;

    if range > 0.0 {
        for s in &mut scores {
            s.score = (s.score - min_score) / range;
        }
    } else {
        for s in &mut scores {
            s.score = 0.5;
        }
    }

    scores.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scores
}

fn entry_point_score(filename: &str, project_type: ProjectType) -> f64 {
    match project_type {
        ProjectType::Rust if filename == "main.rs" || filename == "lib.rs" => 0.5,
        ProjectType::Node if filename == "index.js" || filename == "index.ts" => 0.5,
        ProjectType::Python if filename == "__init__.py" || filename == "main.py" => 0.5,
        ProjectType::Go if filename == "main.go" => 0.5,
        _ if filename == "main.rs" || filename == "lib.rs" || filename == "mod.rs" => 0.3,
        _ => 0.0,
    }
}

fn config_file_score(filename: &str) -> f64 {
    match filename {
        "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "pom.xml" => 0.4,
        "tsconfig.json" | "webpack.config.js" | "vite.config.ts" => 0.2,
        ".gitignore" | ".editorconfig" => 0.1,
        _ => 0.0,
    }
}

fn test_file_penalty(filename: &str, path: &Path) -> f64 {
    let path_str = path.to_string_lossy();
    if filename.starts_with("test_")
        || filename.ends_with("_test.rs")
        || filename.ends_with(".test.js")
        || filename.ends_with(".test.ts")
        || filename.ends_with("_test.go")
        || path_str.contains("tests/")
        || path_str.contains("test/")
        || path_str.contains("__tests__/")
    {
        -0.2
    } else {
        0.0
    }
}

// ── Project summary generation ──────────────────────────────────────

/// A summary of the project for injection into the system prompt.
#[derive(Debug, Clone)]
pub struct ProjectSummary {
    /// Detected project type(s).
    pub project_types: Vec<ProjectType>,
    /// Project root directory.
    pub root: PathBuf,
    /// Key files (top-scored).
    pub key_files: Vec<PathBuf>,
    /// Dependency count (prod + dev).
    pub dependency_count: usize,
    /// Total source file count.
    pub source_file_count: usize,
    /// Optional project name (from manifest).
    pub name: Option<String>,
}

impl ProjectSummary {
    /// Format the summary as a system prompt section.
    #[must_use]
    pub fn to_prompt_section(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Project Context\n");

        if let Some(name) = &self.name {
            let _ = writeln!(out, "- Project: {name}");
        }

        let types: Vec<String> = self
            .project_types
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        let _ = writeln!(out, "- Type: {}", types.join(", "));
        let _ = writeln!(out, "- Root: {}", self.root.display());
        let _ = writeln!(out, "- Source files: {}", self.source_file_count);
        let _ = writeln!(out, "- Dependencies: {}", self.dependency_count);

        if !self.key_files.is_empty() {
            let _ = writeln!(out, "\nKey files:");
            for f in &self.key_files {
                let _ = writeln!(out, "  - {}", f.display());
            }
        }

        let _ = writeln!(out);
        out
    }
}

/// Analyze a project directory and produce a summary.
///
/// This performs a lightweight scan (no compilation or external tools).
#[must_use]
pub fn analyze_project(project_dir: &Path) -> ProjectSummary {
    let project_types = detect_project_type(project_dir);
    let primary_type = project_types
        .first()
        .copied()
        .unwrap_or(ProjectType::Unknown);

    // Count source files and collect paths (up to a limit)
    let mut source_files = Vec::new();
    collect_source_files(project_dir, &mut source_files, 0, 5);

    let source_file_count = source_files.len();

    // Score and pick top files
    let scored = score_files(&source_files, primary_type);
    let key_files: Vec<PathBuf> = scored.into_iter().take(10).map(|s| s.path).collect();

    // Try to read project name
    let name = read_project_name(project_dir, primary_type);

    // Count dependencies
    let dependency_count = count_dependencies(project_dir, primary_type);

    ProjectSummary {
        project_types,
        root: project_dir.to_path_buf(),
        key_files,
        dependency_count,
        source_file_count,
        name,
    }
}

/// Recursively collect source files up to `max_depth`.
fn collect_source_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize, max_depth: usize) {
    if depth > max_depth || files.len() > 500 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common non-source dirs
        if name.starts_with('.')
            || name == "node_modules"
            || name == "target"
            || name == "dist"
            || name == "build"
            || name == "__pycache__"
            || name == "vendor"
        {
            continue;
        }

        if path.is_dir() {
            collect_source_files(&path, files, depth + 1, max_depth);
        } else if is_source_file(&name) {
            files.push(path);
        }
    }
}

fn is_source_file(name: &str) -> bool {
    let source_exts = [
        ".rs", ".js", ".ts", ".jsx", ".tsx", ".py", ".go", ".java", ".kt", ".cs", ".rb", ".php",
        ".swift", ".c", ".cpp", ".h", ".hpp",
    ];
    let config_files = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "Gemfile",
        "composer.json",
    ];

    source_exts.iter().any(|ext| name.ends_with(ext)) || config_files.contains(&name)
}

fn read_project_name(dir: &Path, project_type: ProjectType) -> Option<String> {
    match project_type {
        ProjectType::Rust => {
            let content = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("name") {
                    let rest = rest.trim().strip_prefix('=')?.trim().trim_matches('"');
                    return Some(rest.to_string());
                }
            }
            None
        }
        ProjectType::Node => {
            let content = std::fs::read_to_string(dir.join("package.json")).ok()?;
            let val: serde_json::Value = serde_json::from_str(&content).ok()?;
            val.get("name")?.as_str().map(String::from)
        }
        _ => None,
    }
}

fn count_dependencies(dir: &Path, project_type: ProjectType) -> usize {
    match project_type {
        ProjectType::Rust => {
            let Ok(content) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
                return 0;
            };
            parse_cargo_deps(&content).len()
        }
        ProjectType::Node => {
            let Ok(content) = std::fs::read_to_string(dir.join("package.json")) else {
                return 0;
            };
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
                return 0;
            };
            let deps = val
                .get("dependencies")
                .and_then(|v| v.as_object())
                .map_or(0, serde_json::Map::len);
            let dev = val
                .get("devDependencies")
                .and_then(|v| v.as_object())
                .map_or(0, serde_json::Map::len);
            deps + dev
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Project type detection ──────────────────────────────────────

    #[test]
    fn detect_rust_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let types = detect_project_type(dir.path());
        assert_eq!(types, vec![ProjectType::Rust]);
    }

    #[test]
    fn detect_node_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"test"}"#).unwrap();

        let types = detect_project_type(dir.path());
        assert_eq!(types, vec![ProjectType::Node]);
    }

    #[test]
    fn detect_python_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]").unwrap();

        let types = detect_project_type(dir.path());
        assert_eq!(types, vec![ProjectType::Python]);
    }

    #[test]
    fn detect_go_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com").unwrap();

        let types = detect_project_type(dir.path());
        assert_eq!(types, vec![ProjectType::Go]);
    }

    #[test]
    fn detect_multi_type_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let types = detect_project_type(dir.path());
        assert!(types.contains(&ProjectType::Rust));
        assert!(types.contains(&ProjectType::Node));
    }

    #[test]
    fn detect_unknown_project() {
        let dir = tempfile::tempdir().unwrap();
        let types = detect_project_type(dir.path());
        assert_eq!(types, vec![ProjectType::Unknown]);
    }

    // ── ProjectType Display ─────────────────────────────────────────

    #[test]
    fn project_type_display() {
        assert_eq!(ProjectType::Rust.to_string(), "Rust");
        assert_eq!(ProjectType::Node.to_string(), "Node.js");
        assert_eq!(ProjectType::Python.to_string(), "Python");
        assert_eq!(ProjectType::Unknown.to_string(), "Unknown");
    }

    // ── Cargo dependency parsing ────────────────────────────────────

    #[test]
    fn parse_cargo_deps_basic() {
        let toml = r#"
[package]
name = "test"

[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
tempfile = "3.0"
"#;
        let deps = parse_cargo_deps(toml);
        assert_eq!(deps.len(), 3);

        let serde = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version.as_deref(), Some("1.0"));
        assert!(!serde.dev);

        let tempfile = deps.iter().find(|d| d.name == "tempfile").unwrap();
        assert!(tempfile.dev);
    }

    #[test]
    fn parse_cargo_deps_workspace() {
        let toml = r#"
[dependencies]
serde.workspace = true
tokio.workspace = true
"#;
        let deps = parse_cargo_deps(toml);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[1].name, "tokio");
    }

    #[test]
    fn parse_cargo_deps_empty() {
        let deps = parse_cargo_deps("");
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_cargo_deps_no_deps_section() {
        let toml = "[package]\nname = \"test\"";
        let deps = parse_cargo_deps(toml);
        assert!(deps.is_empty());
    }

    // ── Dependency graph ────────────────────────────────────────────

    #[test]
    fn dep_graph_basic() {
        let mut graph = DependencyGraph::new();
        graph.add_package(
            "app",
            vec![
                Dependency {
                    name: "serde".into(),
                    version: Some("1.0".into()),
                    dev: false,
                },
                Dependency {
                    name: "tokio".into(),
                    version: None,
                    dev: false,
                },
            ],
        );

        assert_eq!(graph.package_count(), 1);
        assert_eq!(graph.total_deps(), 2);

        let deps = graph.deps_for("app").unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "serde");
    }

    #[test]
    fn dep_graph_reverse_deps() {
        let mut graph = DependencyGraph::new();
        graph.add_package(
            "app",
            vec![Dependency {
                name: "core".into(),
                version: None,
                dev: false,
            }],
        );
        graph.add_package(
            "tools",
            vec![Dependency {
                name: "core".into(),
                version: None,
                dev: false,
            }],
        );
        graph.add_package("core", vec![]);

        let rev = graph.reverse_deps("core");
        assert_eq!(rev.len(), 2);
        assert!(rev.contains(&"app"));
        assert!(rev.contains(&"tools"));
    }

    #[test]
    fn dep_graph_no_reverse_deps() {
        let graph = DependencyGraph::new();
        assert!(graph.reverse_deps("unknown").is_empty());
    }

    #[test]
    fn dep_graph_deps_for_unknown() {
        let graph = DependencyGraph::new();
        assert!(graph.deps_for("missing").is_none());
    }

    // ── File importance scoring ─────────────────────────────────────

    #[test]
    fn score_files_empty() {
        let scores = score_files(&[], ProjectType::Rust);
        assert!(scores.is_empty());
    }

    #[test]
    fn score_files_entry_point_ranked_high() {
        let files = vec![
            PathBuf::from("src/utils.rs"),
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/lib.rs"),
        ];
        let scores = score_files(&files, ProjectType::Rust);
        // main.rs and lib.rs should be ranked higher than utils.rs
        assert!(scores[0].path.file_name().unwrap() != "utils.rs");
    }

    #[test]
    fn score_files_config_ranked_high() {
        let files = vec![
            PathBuf::from("src/deep/nested/file.rs"),
            PathBuf::from("Cargo.toml"),
        ];
        let scores = score_files(&files, ProjectType::Rust);
        assert_eq!(
            scores[0].path.file_name().unwrap().to_string_lossy(),
            "Cargo.toml"
        );
    }

    #[test]
    fn score_files_test_ranked_lower() {
        let files = vec![
            PathBuf::from("src/core.rs"),
            PathBuf::from("tests/test_core.rs"),
        ];
        let scores = score_files(&files, ProjectType::Rust);
        assert_eq!(
            scores[0].path.file_name().unwrap().to_string_lossy(),
            "core.rs"
        );
    }

    #[test]
    fn score_files_all_same_get_half() {
        let files = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        let scores = score_files(&files, ProjectType::Unknown);
        // Both have same depth and no special scores, so normalized to 0.5
        assert!((scores[0].score - 0.5).abs() < f64::EPSILON);
    }

    // ── Project summary ─────────────────────────────────────────────

    #[test]
    fn analyze_rust_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-app\"\n\n[dependencies]\nserde = \"1.0\"\n",
        )
        .unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(src.join("lib.rs"), "pub mod foo;").unwrap();

        let summary = analyze_project(dir.path());
        assert!(summary.project_types.contains(&ProjectType::Rust));
        assert_eq!(summary.name.as_deref(), Some("my-app"));
        assert!(summary.dependency_count >= 1);
        assert!(summary.source_file_count >= 2);
    }

    #[test]
    fn analyze_node_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"my-app","dependencies":{"express":"4.0"},"devDependencies":{"jest":"29"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("index.js"), "module.exports = {}").unwrap();

        let summary = analyze_project(dir.path());
        assert!(summary.project_types.contains(&ProjectType::Node));
        assert_eq!(summary.name.as_deref(), Some("my-app"));
        assert_eq!(summary.dependency_count, 2);
    }

    #[test]
    fn analyze_empty_project() {
        let dir = tempfile::tempdir().unwrap();
        let summary = analyze_project(dir.path());
        assert_eq!(summary.project_types, vec![ProjectType::Unknown]);
        assert!(summary.name.is_none());
        assert_eq!(summary.dependency_count, 0);
    }

    #[test]
    fn summary_to_prompt_section() {
        let summary = ProjectSummary {
            project_types: vec![ProjectType::Rust],
            root: PathBuf::from("/tmp/project"),
            key_files: vec![PathBuf::from("src/main.rs")],
            dependency_count: 15,
            source_file_count: 42,
            name: Some("my-project".into()),
        };

        let section = summary.to_prompt_section();
        assert!(section.contains("Project Context"));
        assert!(section.contains("my-project"));
        assert!(section.contains("Rust"));
        assert!(section.contains("42"));
        assert!(section.contains("15"));
        assert!(section.contains("src/main.rs"));
    }

    #[test]
    fn summary_to_prompt_section_no_name() {
        let summary = ProjectSummary {
            project_types: vec![ProjectType::Unknown],
            root: PathBuf::from("."),
            key_files: vec![],
            dependency_count: 0,
            source_file_count: 0,
            name: None,
        };

        let section = summary.to_prompt_section();
        assert!(section.contains("Unknown"));
        assert!(!section.contains("Project:"));
    }

    // ── Helper function tests ───────────────────────────────────────

    #[test]
    fn is_source_file_recognizes_rust() {
        assert!(is_source_file("main.rs"));
        assert!(is_source_file("lib.rs"));
        assert!(!is_source_file("data.json"));
    }

    #[test]
    fn is_source_file_recognizes_config() {
        assert!(is_source_file("Cargo.toml"));
        assert!(is_source_file("package.json"));
    }

    #[test]
    fn is_source_file_rejects_non_source() {
        assert!(!is_source_file("image.png"));
        assert!(!is_source_file("readme.md"));
        assert!(!is_source_file("data.csv"));
    }

    #[test]
    fn entry_point_score_rust() {
        assert!(entry_point_score("main.rs", ProjectType::Rust) > 0.0);
        assert!(entry_point_score("lib.rs", ProjectType::Rust) > 0.0);
        assert_eq!(entry_point_score("utils.rs", ProjectType::Rust), 0.0);
    }

    #[test]
    fn entry_point_score_node() {
        assert!(entry_point_score("index.js", ProjectType::Node) > 0.0);
        assert!(entry_point_score("index.ts", ProjectType::Node) > 0.0);
    }

    #[test]
    fn config_file_score_values() {
        assert!(config_file_score("Cargo.toml") > 0.0);
        assert!(config_file_score("package.json") > 0.0);
        assert_eq!(config_file_score("random.txt"), 0.0);
    }

    #[test]
    fn test_file_penalty_values() {
        assert!(test_file_penalty("test_main.rs", Path::new("test_main.rs")) < 0.0);
        assert!(test_file_penalty("app.rs", Path::new("tests/app.rs")) < 0.0);
        assert_eq!(test_file_penalty("app.rs", Path::new("src/app.rs")), 0.0);
    }
}
