//! REPL slash-command parsing for conversation management.
//!
//! Handles `/undo`, `/branch`, and `/fork` command parsing.

/// A parsed REPL command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// `/undo [N]` — rollback the last N turns (default 1).
    Undo { turns: usize },
    /// `/branch` — list all branches.
    BranchList,
    /// `/branch <name>` — switch to the named branch.
    BranchSwitch { name: String },
    /// `/fork [label]` — fork a new branch from the current position.
    Fork { label: Option<String> },
    /// Not a recognized command; treat as normal user input.
    NotACommand,
}

impl ReplCommand {
    /// Parse a user input string into a `ReplCommand`.
    ///
    /// Returns `NotACommand` if the input doesn't start with a recognized
    /// slash command.
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();

        if let Some(rest) = trimmed.strip_prefix("/undo") {
            let rest = rest.trim();
            if rest.is_empty() {
                return Self::Undo { turns: 1 };
            }
            if let Ok(n) = rest.parse::<usize>() {
                return Self::Undo { turns: n };
            }
            // Invalid number — treat as not a command
            return Self::NotACommand;
        }

        if let Some(rest) = trimmed.strip_prefix("/branch") {
            let rest = rest.trim();
            if rest.is_empty() {
                return Self::BranchList;
            }
            return Self::BranchSwitch {
                name: rest.to_string(),
            };
        }

        if let Some(rest) = trimmed.strip_prefix("/fork") {
            let rest = rest.trim();
            let label = if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            };
            return Self::Fork { label };
        }

        Self::NotACommand
    }
}

/// Result of executing a REPL command.
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// Human-readable output to display to the user.
    pub output: String,
    /// Whether the command was successful.
    pub success: bool,
}

impl CommandResult {
    #[must_use]
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            success: true,
        }
    }

    #[must_use]
    pub fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            success: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_undo_default() {
        assert_eq!(ReplCommand::parse("/undo"), ReplCommand::Undo { turns: 1 });
    }

    #[test]
    fn parse_undo_with_number() {
        assert_eq!(
            ReplCommand::parse("/undo 3"),
            ReplCommand::Undo { turns: 3 }
        );
    }

    #[test]
    fn parse_undo_with_whitespace() {
        assert_eq!(
            ReplCommand::parse("  /undo  2  "),
            ReplCommand::Undo { turns: 2 }
        );
    }

    #[test]
    fn parse_undo_invalid_number() {
        assert_eq!(ReplCommand::parse("/undo abc"), ReplCommand::NotACommand);
    }

    #[test]
    fn parse_branch_list() {
        assert_eq!(ReplCommand::parse("/branch"), ReplCommand::BranchList);
    }

    #[test]
    fn parse_branch_switch() {
        assert_eq!(
            ReplCommand::parse("/branch my-branch"),
            ReplCommand::BranchSwitch {
                name: "my-branch".into()
            }
        );
    }

    #[test]
    fn parse_branch_switch_with_whitespace() {
        assert_eq!(
            ReplCommand::parse("  /branch  alt  "),
            ReplCommand::BranchSwitch { name: "alt".into() }
        );
    }

    #[test]
    fn parse_fork_no_label() {
        assert_eq!(
            ReplCommand::parse("/fork"),
            ReplCommand::Fork { label: None }
        );
    }

    #[test]
    fn parse_fork_with_label() {
        assert_eq!(
            ReplCommand::parse("/fork experiment"),
            ReplCommand::Fork {
                label: Some("experiment".into())
            }
        );
    }

    #[test]
    fn parse_not_a_command() {
        assert_eq!(ReplCommand::parse("hello world"), ReplCommand::NotACommand);
    }

    #[test]
    fn parse_unknown_slash_command() {
        assert_eq!(ReplCommand::parse("/unknown"), ReplCommand::NotACommand);
    }

    #[test]
    fn parse_empty_string() {
        assert_eq!(ReplCommand::parse(""), ReplCommand::NotACommand);
    }

    #[test]
    fn command_result_ok() {
        let r = CommandResult::ok("done");
        assert!(r.success);
        assert_eq!(r.output, "done");
    }

    #[test]
    fn command_result_err() {
        let r = CommandResult::err("failed");
        assert!(!r.success);
        assert_eq!(r.output, "failed");
    }
}
