//! Shell completion script generation.
//!
//! Uses `clap_complete` to generate completion scripts for bash, zsh, fish,
//! and `PowerShell`. Output is written to stdout so users can redirect to the
//! appropriate shell config file.

use clap::CommandFactory;
use clap_complete::{Shell, generate};
use std::io;

/// Generate a completion script for the given shell and write it to stdout.
pub fn generate_completions(shell: Shell) {
    let mut cmd = crate::Cli::command();
    generate(shell, &mut cmd, "crab", &mut io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_bash_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        generate(Shell::Bash, &mut cmd, "crab", &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("crab"));
        assert!(!output.is_empty());
    }

    #[test]
    fn generate_zsh_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        generate(Shell::Zsh, &mut cmd, "crab", &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("crab"));
    }

    #[test]
    fn generate_fish_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        generate(Shell::Fish, &mut cmd, "crab", &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("crab"));
    }

    #[test]
    fn generate_powershell_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        generate(Shell::PowerShell, &mut cmd, "crab", &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("crab"));
    }
}
