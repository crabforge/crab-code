pub mod bench;
pub mod ci;
pub mod coverage;
pub mod release;

use std::process::Command;

/// Find the workspace root (directory containing the top-level Cargo.toml).
pub fn workspace_root() -> Result<std::path::PathBuf, String> {
    // xtask binary lives at <root>/xtask/src/main.rs — the workspace root is
    // the parent of the directory containing Cargo.toml for xtask. We use
    // `cargo locate-project --workspace` for a robust answer.
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if !output.status.success() {
        return Err("cargo locate-project failed".into());
    }
    let path = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "could not determine workspace root".into())
}

/// Run a command, printing it first, and return an error on failure.
pub fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    println!("+ {cmd} {}", args.join(" "));
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run `{cmd}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{cmd} {}` exited with {}", args.join(" "), status))
    }
}

/// Run a command and capture its stdout.
pub fn run_cmd_output(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run `{cmd}`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`{cmd}` failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_finds_cargo_toml() {
        let root = workspace_root().unwrap();
        assert!(root.join("Cargo.toml").exists());
        assert!(root.join("crates").is_dir());
    }

    #[test]
    fn run_cmd_success() {
        // `cargo --version` should always succeed
        let result = run_cmd("cargo", &["--version"]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_cmd_failure() {
        let result = run_cmd("cargo", &["nonexistent-subcommand-xyz"]);
        assert!(result.is_err());
    }

    #[test]
    fn run_cmd_output_captures_stdout() {
        let out = run_cmd_output("cargo", &["--version"]).unwrap();
        assert!(out.starts_with("cargo"));
    }
}
