//! Shell completion script generation.
//!
//! Provides a standalone utility to generate shell completion scripts for
//! the `crab` CLI using `clap_complete`. This module complements the
//! `Completion` subcommand in `main.rs` by exposing a reusable function
//! and a crate-local [`Shell`] enum.

use std::io::Write;

/// Supported shells for completion generation.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// GNU Bash.
    Bash,
    /// Z shell.
    Zsh,
    /// Fish shell.
    Fish,
    /// PowerShell (cross-platform).
    PowerShell,
}

impl Shell {
    /// Convert to the `clap_complete` [`Shell`](clap_complete::Shell) variant.
    fn to_clap(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Zsh => clap_complete::Shell::Zsh,
            Self::Fish => clap_complete::Shell::Fish,
            Self::PowerShell => clap_complete::Shell::PowerShell,
        }
    }

    /// Parse a shell name (case-insensitive).
    ///
    /// Returns `None` for unrecognised names.
    #[allow(dead_code)]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            "powershell" | "pwsh" => Some(Self::PowerShell),
            _ => None,
        }
    }

    /// The canonical lowercase name of this shell.
    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }

    /// All supported shell variants.
    #[allow(dead_code)]
    pub fn all() -> &'static [Shell] {
        &[Self::Bash, Self::Zsh, Self::Fish, Self::PowerShell]
    }
}

impl std::fmt::Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Generate shell completion scripts for the `crab` binary.
///
/// The completions are written to the provided writer. Pass a clap
/// [`Command`](clap::Command) that describes the full CLI so that all
/// subcommands and flags are included.
///
/// # Errors
///
/// Returns an error if writing to the output fails.
#[allow(dead_code)]
pub fn generate_completions<W: Write>(
    shell: Shell,
    cmd: &mut clap::Command,
    writer: &mut W,
) -> std::io::Result<()> {
    clap_complete::generate(shell.to_clap(), cmd, "crab", writer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_from_name_known() {
        assert_eq!(Shell::from_name("bash"), Some(Shell::Bash));
        assert_eq!(Shell::from_name("ZSH"), Some(Shell::Zsh));
        assert_eq!(Shell::from_name("Fish"), Some(Shell::Fish));
        assert_eq!(Shell::from_name("PowerShell"), Some(Shell::PowerShell));
        assert_eq!(Shell::from_name("pwsh"), Some(Shell::PowerShell));
    }

    #[test]
    fn shell_from_name_unknown() {
        assert_eq!(Shell::from_name("nushell"), None);
        assert_eq!(Shell::from_name(""), None);
    }

    #[test]
    fn shell_name_roundtrip() {
        for shell in Shell::all() {
            assert_eq!(Shell::from_name(shell.name()), Some(*shell));
        }
    }

    #[test]
    fn shell_display() {
        assert_eq!(Shell::Bash.to_string(), "bash");
        assert_eq!(Shell::PowerShell.to_string(), "powershell");
    }

    #[test]
    fn generate_completions_produces_output() {
        let mut cmd = clap::Command::new("crab")
            .subcommand(clap::Command::new("doctor"))
            .subcommand(clap::Command::new("config"))
            .subcommand(clap::Command::new("session"));

        let mut buf = Vec::new();
        generate_completions(Shell::Bash, &mut cmd, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(!output.is_empty());
        assert!(output.contains("crab"));
    }

    #[test]
    fn all_shells_generate_without_error() {
        for shell in Shell::all() {
            let mut cmd = clap::Command::new("crab").subcommand(clap::Command::new("test"));
            let mut buf = Vec::new();
            generate_completions(*shell, &mut cmd, &mut buf).unwrap();
            assert!(!buf.is_empty(), "shell {shell} produced empty output");
        }
    }
}
