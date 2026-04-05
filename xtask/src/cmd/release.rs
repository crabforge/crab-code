//! `cargo xtask release` — bump workspace version, update CHANGELOG, and create a git tag.

use std::fs;
use std::path::Path;

use clap::Args;

use super::{run_cmd, run_cmd_output, workspace_root};

/// Semver bump level.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum BumpLevel {
    Patch,
    Minor,
    Major,
}

#[derive(Args)]
pub struct ReleaseArgs {
    /// Which semver component to bump.
    #[arg(value_enum, default_value_t = BumpLevel::Patch)]
    pub bump: BumpLevel,
    /// Actually create the tag and commit. Without this flag, only a dry-run is performed.
    #[arg(long)]
    pub execute: bool,
}

pub fn run(args: &ReleaseArgs) -> Result<(), String> {
    let root = workspace_root()?;
    let cargo_toml_path = root.join("Cargo.toml");

    // Read current version from workspace Cargo.toml
    let content = fs::read_to_string(&cargo_toml_path)
        .map_err(|e| format!("failed to read Cargo.toml: {e}"))?;
    let current = extract_workspace_version(&content)?;

    let next = bump_version(&current, args.bump)?;

    println!("current version: {current}");
    println!("next version:    {next}");
    println!("bump level:      {:?}", args.bump);

    if !args.execute {
        println!("\ndry-run mode — pass --execute to apply changes");
        return Ok(());
    }

    // Update workspace version in Cargo.toml
    let new_content = set_workspace_version(&content, &next)?;
    fs::write(&cargo_toml_path, new_content)
        .map_err(|e| format!("failed to write Cargo.toml: {e}"))?;
    println!("updated Cargo.toml workspace version to {next}");

    // Update CHANGELOG.md
    update_changelog(&root, &next)?;

    // Git commit and tag
    run_cmd("git", &["add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"])?;
    let msg = format!("release: v{next}");
    run_cmd("git", &["commit", "-m", &msg])?;

    let tag = format!("v{next}");
    run_cmd("git", &["tag", "-a", &tag, "-m", &msg])?;
    println!("\ncreated tag {tag}");

    Ok(())
}

/// Extract `version = "x.y.z"` from `[workspace.package]`.
pub fn extract_workspace_version(cargo_toml: &str) -> Result<String, String> {
    let doc = cargo_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("failed to parse Cargo.toml: {e}"))?;
    let version = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .ok_or("could not find workspace.package.version")?;
    Ok(version.to_string())
}

/// Set `workspace.package.version` in a Cargo.toml string, preserving formatting.
pub fn set_workspace_version(cargo_toml: &str, new_version: &str) -> Result<String, String> {
    let mut doc = cargo_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("failed to parse Cargo.toml: {e}"))?;
    doc["workspace"]["package"]["version"] = toml_edit::value(new_version);
    Ok(doc.to_string())
}

/// Bump a semver version string.
pub fn bump_version(version: &str, level: BumpLevel) -> Result<String, String> {
    let parts: Vec<u64> = version
        .split('.')
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| format!("invalid version component: {s}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if parts.len() != 3 {
        return Err(format!("expected semver x.y.z, got: {version}"));
    }

    let (major, minor, patch) = (parts[0], parts[1], parts[2]);
    let (major, minor, patch) = match level {
        BumpLevel::Patch => (major, minor, patch + 1),
        BumpLevel::Minor => (major, minor + 1, 0),
        BumpLevel::Major => (major + 1, 0, 0),
    };

    Ok(format!("{major}.{minor}.{patch}"))
}

/// Prepend a release header to CHANGELOG.md (or create it).
fn update_changelog(root: &Path, version: &str) -> Result<(), String> {
    let changelog_path = root.join("CHANGELOG.md");
    let today = today_str();

    // Collect git log since last tag (if any)
    let log = recent_git_log();

    let header = format!("## v{version} ({today})\n\n{log}\n");

    let existing = fs::read_to_string(&changelog_path).unwrap_or_default();
    let new_content = if existing.is_empty() {
        format!("# Changelog\n\n{header}")
    } else {
        // Insert after the first line (the `# Changelog` heading)
        match existing.find('\n') {
            Some(pos) => {
                let (head, tail) = existing.split_at(pos + 1);
                format!("{head}\n{header}{tail}")
            }
            None => format!("{existing}\n\n{header}"),
        }
    };

    fs::write(&changelog_path, new_content)
        .map_err(|e| format!("failed to write CHANGELOG.md: {e}"))?;
    println!("updated CHANGELOG.md");
    Ok(())
}

fn today_str() -> String {
    // Use git to get today's date in a portable way
    run_cmd_output("git", &["log", "-1", "--format=%cd", "--date=short"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn recent_git_log() -> String {
    // Try to get commits since last tag
    let tag_result = run_cmd_output("git", &["describe", "--tags", "--abbrev=0"]);
    let range = match &tag_result {
        Ok(tag) => format!("{}..HEAD", tag.trim()),
        Err(_) => "HEAD~20..HEAD".to_string(),
    };

    run_cmd_output("git", &["log", "--oneline", "--no-decorate", &range])
        .unwrap_or_else(|_| "- Initial release\n".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_patch() {
        assert_eq!(bump_version("0.1.0", BumpLevel::Patch).unwrap(), "0.1.1");
    }

    #[test]
    fn bump_minor() {
        assert_eq!(bump_version("0.1.5", BumpLevel::Minor).unwrap(), "0.2.0");
    }

    #[test]
    fn bump_major() {
        assert_eq!(bump_version("1.2.3", BumpLevel::Major).unwrap(), "2.0.0");
    }

    #[test]
    fn bump_invalid_version() {
        assert!(bump_version("not.a.ver", BumpLevel::Patch).is_err());
        assert!(bump_version("1.2", BumpLevel::Patch).is_err());
    }

    #[test]
    fn extract_version_from_cargo_toml() {
        let toml = r#"
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
"#;
        assert_eq!(extract_workspace_version(toml).unwrap(), "0.1.0");
    }

    #[test]
    fn set_version_preserves_structure() {
        let toml = r#"[workspace.package]
version = "0.1.0"
edition = "2024"
"#;
        let result = set_workspace_version(toml, "0.2.0").unwrap();
        assert!(result.contains("\"0.2.0\""));
        assert!(result.contains("edition"));
    }

    #[test]
    fn extract_version_missing() {
        let toml = "[package]\nname = \"foo\"\n";
        assert!(extract_workspace_version(toml).is_err());
    }
}
