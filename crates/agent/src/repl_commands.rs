//! REPL slash-command parsing and execution for conversation management.
//!
//! Handles `/undo`, `/branch`, and `/fork` commands that operate on the
//! conversation tree within an agent session.

use crate::conversation_tree::{BranchId, ConversationTree};

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
    fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            success: true,
        }
    }

    fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            success: false,
        }
    }
}

/// Execute a parsed REPL command against a conversation tree.
///
/// Returns `None` if the command is `NotACommand` (meaning the input
/// should be processed as normal user input).
#[must_use]
pub fn execute_command(cmd: &ReplCommand, tree: &mut ConversationTree) -> Option<CommandResult> {
    match cmd {
        ReplCommand::NotACommand => None,
        ReplCommand::Undo { turns } => Some(exec_undo(tree, *turns)),
        ReplCommand::BranchList => Some(exec_branch_list(tree)),
        ReplCommand::BranchSwitch { name } => Some(exec_branch_switch(tree, name)),
        ReplCommand::Fork { label } => Some(exec_fork(tree, label.clone())),
    }
}

fn exec_undo(tree: &mut ConversationTree, turns: usize) -> CommandResult {
    match tree.rollback_turns(turns) {
        Ok(0) => CommandResult::ok("Nothing to undo."),
        Ok(n) => CommandResult::ok(format!(
            "Rolled back {n} message(s). Conversation now has {} message(s).",
            tree.current_depth()
        )),
        Err(e) => CommandResult::err(format!("Undo failed: {e}")),
    }
}

fn exec_branch_list(tree: &ConversationTree) -> CommandResult {
    let active = tree.active_branch();
    let mut lines = Vec::new();
    let mut branch_ids: Vec<_> = tree.branch_ids();
    branch_ids.sort_by_key(|id| id.as_str().to_string());

    for id in branch_ids {
        let marker = if id == active { "* " } else { "  " };
        let depth = tree.branch_messages(id).len();
        let branch = tree.get_branch(id);
        let label = branch.and_then(|b| b.label.as_deref()).unwrap_or("");
        if label.is_empty() {
            lines.push(format!("{marker}{} ({depth} messages)", id.as_str()));
        } else {
            lines.push(format!(
                "{marker}{} ({depth} messages) — {label}",
                id.as_str()
            ));
        }
    }

    if lines.is_empty() {
        CommandResult::ok("No branches.")
    } else {
        CommandResult::ok(lines.join("\n"))
    }
}

fn exec_branch_switch(tree: &mut ConversationTree, name: &str) -> CommandResult {
    let id = BranchId::new(name);
    match tree.switch_branch(&id) {
        Ok(()) => CommandResult::ok(format!(
            "Switched to branch '{}'. {} message(s) in history.",
            name,
            tree.current_depth()
        )),
        Err(e) => CommandResult::err(format!("Switch failed: {e}")),
    }
}

fn exec_fork(tree: &mut ConversationTree, label: Option<String>) -> CommandResult {
    match tree.fork_here(label) {
        Ok(id) => CommandResult::ok(format!(
            "Created and switched to new branch '{}'. {} message(s) in history.",
            id.as_str(),
            tree.current_depth()
        )),
        Err(e) => CommandResult::err(format!("Fork failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::message::Message;

    // ── Parsing ─────────────────────────────────────────────────────

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

    // ── Execution ───────────────────────────────────────────────────

    fn make_tree_with_turns(n: usize) -> ConversationTree {
        let mut tree = ConversationTree::new();
        for i in 0..n {
            tree.push(Message::user(format!("Q{i}")));
            tree.push(Message::assistant(format!("A{i}")));
        }
        tree
    }

    #[test]
    fn exec_not_a_command_returns_none() {
        let mut tree = ConversationTree::new();
        assert!(execute_command(&ReplCommand::NotACommand, &mut tree).is_none());
    }

    #[test]
    fn exec_undo_on_empty() {
        let mut tree = ConversationTree::new();
        let result = execute_command(&ReplCommand::Undo { turns: 1 }, &mut tree).unwrap();
        assert!(result.success);
        assert_eq!(result.output, "Nothing to undo.");
    }

    #[test]
    fn exec_undo_one_turn() {
        let mut tree = make_tree_with_turns(3);
        assert_eq!(tree.current_depth(), 6);

        let result = execute_command(&ReplCommand::Undo { turns: 1 }, &mut tree).unwrap();
        assert!(result.success);
        assert!(result.output.contains("2 message(s)"));
        assert_eq!(tree.current_depth(), 4);
    }

    #[test]
    fn exec_undo_multiple_turns() {
        let mut tree = make_tree_with_turns(3);
        let result = execute_command(&ReplCommand::Undo { turns: 2 }, &mut tree).unwrap();
        assert!(result.success);
        assert_eq!(tree.current_depth(), 2);
    }

    #[test]
    fn exec_branch_list_shows_main() {
        let tree = ConversationTree::new();
        let result = execute_command(&ReplCommand::BranchList, &mut { tree }).unwrap();
        assert!(result.success);
        assert!(result.output.contains("main"));
        assert!(result.output.contains("*")); // active marker
    }

    #[test]
    fn exec_branch_list_multiple() {
        let mut tree = make_tree_with_turns(1);
        tree.create_branch(0, Some("alt-path".into())).unwrap();

        let result = execute_command(&ReplCommand::BranchList, &mut tree).unwrap();
        assert!(result.success);
        assert!(result.output.contains("main"));
        assert!(result.output.contains("branch-1"));
        assert!(result.output.contains("alt-path"));
    }

    #[test]
    fn exec_branch_switch_success() {
        let mut tree = make_tree_with_turns(1);
        let branch_id = tree.create_branch(0, None).unwrap();

        let result = execute_command(
            &ReplCommand::BranchSwitch {
                name: branch_id.as_str().to_string(),
            },
            &mut tree,
        )
        .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Switched to branch"));
    }

    #[test]
    fn exec_branch_switch_not_found() {
        let mut tree = ConversationTree::new();
        let result = execute_command(
            &ReplCommand::BranchSwitch {
                name: "nonexistent".into(),
            },
            &mut tree,
        )
        .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Switch failed"));
    }

    #[test]
    fn exec_fork_success() {
        let mut tree = make_tree_with_turns(1);
        let result = execute_command(&ReplCommand::Fork { label: None }, &mut tree).unwrap();
        assert!(result.success);
        assert!(result.output.contains("Created and switched"));
        assert_eq!(tree.branch_count(), 2);
    }

    #[test]
    fn exec_fork_with_label() {
        let mut tree = make_tree_with_turns(1);
        let result = execute_command(
            &ReplCommand::Fork {
                label: Some("experiment".into()),
            },
            &mut tree,
        )
        .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Created and switched"));
    }

    #[test]
    fn exec_fork_on_empty_fails() {
        let mut tree = ConversationTree::new();
        let result = execute_command(&ReplCommand::Fork { label: None }, &mut tree).unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Fork failed"));
    }

    // ── Round-trip: fork then switch back ───────────────────────────

    #[test]
    fn fork_and_switch_back_to_main() {
        let mut tree = make_tree_with_turns(2);
        assert_eq!(tree.current_depth(), 4);

        // Fork
        let result = execute_command(&ReplCommand::Fork { label: None }, &mut tree).unwrap();
        assert!(result.success);
        let forked_depth = tree.current_depth();
        assert_eq!(forked_depth, 4);

        // Add messages on fork
        tree.push(Message::user("Q-fork"));
        tree.push(Message::assistant("A-fork"));
        assert_eq!(tree.current_depth(), 6);

        // Switch back to main
        let result = execute_command(
            &ReplCommand::BranchSwitch {
                name: "main".into(),
            },
            &mut tree,
        )
        .unwrap();
        assert!(result.success);
        assert_eq!(tree.current_depth(), 4);
    }

    // ── Undo then fork creates clean branch ─────────────────────────

    #[test]
    fn undo_then_fork() {
        let mut tree = make_tree_with_turns(3);
        assert_eq!(tree.current_depth(), 6);

        // Undo last turn
        let _ = execute_command(&ReplCommand::Undo { turns: 1 }, &mut tree);
        assert_eq!(tree.current_depth(), 4);

        // Fork from here
        let result = execute_command(&ReplCommand::Fork { label: None }, &mut tree).unwrap();
        assert!(result.success);
        assert_eq!(tree.current_depth(), 4);

        // New branch gets new messages
        tree.push(Message::user("Q-alt"));
        tree.push(Message::assistant("A-alt"));
        assert_eq!(tree.current_depth(), 6);
    }

    // ── CommandResult ───────────────────────────────────────────────

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
