//! Conversation branching, rollback, and tree management.
//!
//! Provides a tree-structured conversation history where any message can become
//! a branch point, enabling exploration of alternative conversation paths.

use crab_core::message::Message;
use std::collections::HashMap;

/// Unique identifier for a branch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchId(pub String);

impl BranchId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BranchId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A single node in the conversation tree.
#[derive(Debug, Clone)]
pub struct ConversationNode {
    /// Unique index within the tree's node list.
    pub index: usize,
    /// The message at this node.
    pub message: Message,
    /// Parent node index (None for root).
    pub parent: Option<usize>,
    /// Child node indices.
    pub children: Vec<usize>,
}

/// A named branch: a linear path through the tree ending at a specific node.
#[derive(Debug, Clone)]
pub struct Branch {
    /// Branch identifier.
    pub id: BranchId,
    /// Index of the node where this branch diverged from its parent branch.
    /// None for the main branch.
    pub fork_point: Option<usize>,
    /// The tip (latest node) of this branch.
    pub tip: Option<usize>,
    /// Optional label for the branch.
    pub label: Option<String>,
}

/// The main branch name.
const MAIN_BRANCH: &str = "main";

/// A tree-structured conversation supporting branching and rollback.
///
/// Messages are stored in a flat `Vec<ConversationNode>` with parent/child
/// pointers forming the tree. Named branches track different conversation paths.
#[derive(Debug)]
pub struct ConversationTree {
    /// All nodes in the tree (append-only).
    nodes: Vec<ConversationNode>,
    /// Named branches mapping to their metadata.
    branches: HashMap<BranchId, Branch>,
    /// The currently active branch.
    active_branch: BranchId,
    /// Auto-incrementing counter for generating branch names.
    branch_counter: u64,
}

impl ConversationTree {
    /// Create a new conversation tree with a "main" branch.
    #[must_use]
    pub fn new() -> Self {
        let main_id = BranchId::new(MAIN_BRANCH);
        let mut branches = HashMap::new();
        branches.insert(
            main_id.clone(),
            Branch {
                id: main_id.clone(),
                fork_point: None,
                tip: None,
                label: Some("Main conversation".into()),
            },
        );
        Self {
            nodes: Vec::new(),
            branches,
            active_branch: main_id,
            branch_counter: 0,
        }
    }

    /// Push a message onto the current branch.
    pub fn push(&mut self, message: Message) {
        let parent = self.current_tip();
        let index = self.nodes.len();

        self.nodes.push(ConversationNode {
            index,
            message,
            parent,
            children: Vec::new(),
        });

        // Link parent to child
        if let Some(parent_idx) = parent {
            self.nodes[parent_idx].children.push(index);
        }

        // Update branch tip
        if let Some(branch) = self.branches.get_mut(&self.active_branch) {
            branch.tip = Some(index);
        }
    }

    /// Get the tip node index of the current branch.
    #[must_use]
    pub fn current_tip(&self) -> Option<usize> {
        self.branches.get(&self.active_branch).and_then(|b| b.tip)
    }

    /// Get the currently active branch ID.
    #[must_use]
    pub fn active_branch(&self) -> &BranchId {
        &self.active_branch
    }

    /// Get all messages on the current branch, from root to tip.
    #[must_use]
    pub fn current_messages(&self) -> Vec<&Message> {
        self.messages_to_tip(self.current_tip())
    }

    /// Get all messages from root to the given node (inclusive).
    fn messages_to_tip(&self, tip: Option<usize>) -> Vec<&Message> {
        let Some(tip_idx) = tip else {
            return Vec::new();
        };

        // Walk from tip to root, then reverse
        let mut path = Vec::new();
        let mut current = Some(tip_idx);
        while let Some(idx) = current {
            let node = &self.nodes[idx];
            path.push(&node.message);
            current = node.parent;
        }
        path.reverse();
        path
    }

    /// Create a new branch forking from a specific node index.
    ///
    /// Returns the new branch ID, or an error if the node index is invalid.
    pub fn create_branch(
        &mut self,
        fork_from: usize,
        label: Option<String>,
    ) -> Result<BranchId, BranchError> {
        if fork_from >= self.nodes.len() {
            return Err(BranchError::InvalidNode(fork_from));
        }

        self.branch_counter += 1;
        let name = format!("branch-{}", self.branch_counter);
        let id = BranchId::new(&name);

        self.branches.insert(
            id.clone(),
            Branch {
                id: id.clone(),
                fork_point: Some(fork_from),
                tip: Some(fork_from),
                label,
            },
        );

        Ok(id)
    }

    /// Create a branch forking from a specific node and switch to it.
    pub fn fork_and_switch(
        &mut self,
        fork_from: usize,
        label: Option<String>,
    ) -> Result<BranchId, BranchError> {
        let id = self.create_branch(fork_from, label)?;
        self.active_branch = id.clone();
        Ok(id)
    }

    /// Create a branch from the current tip of the active branch.
    pub fn fork_here(&mut self, label: Option<String>) -> Result<BranchId, BranchError> {
        let tip = self
            .current_tip()
            .ok_or_else(|| BranchError::EmptyBranch(self.active_branch.clone()))?;
        self.fork_and_switch(tip, label)
    }

    /// Switch to an existing branch.
    pub fn switch_branch(&mut self, branch_id: &BranchId) -> Result<(), BranchError> {
        if !self.branches.contains_key(branch_id) {
            return Err(BranchError::BranchNotFound(branch_id.clone()));
        }
        self.active_branch = branch_id.clone();
        Ok(())
    }

    /// Rollback the current branch by removing the last `n` messages.
    ///
    /// Returns the number of messages actually removed (may be less than `n`
    /// if the branch doesn't have that many messages past its fork point).
    pub fn rollback(&mut self, n: usize) -> Result<usize, BranchError> {
        if n == 0 {
            return Ok(0);
        }

        let branch = self
            .branches
            .get(&self.active_branch)
            .ok_or_else(|| BranchError::BranchNotFound(self.active_branch.clone()))?;

        let fork_point = branch.fork_point;
        let Some(mut current_tip) = branch.tip else {
            return Ok(0);
        };

        let mut removed = 0;
        for _ in 0..n {
            // Don't roll back past the fork point
            if Some(current_tip) == fork_point {
                break;
            }

            let parent = self.nodes[current_tip].parent;

            // Remove this node from parent's children
            if let Some(parent_idx) = parent {
                self.nodes[parent_idx]
                    .children
                    .retain(|&c| c != current_tip);
            }

            removed += 1;

            if let Some(p) = parent {
                current_tip = p;
            } else {
                // Rolled back to the very beginning
                if let Some(b) = self.branches.get_mut(&self.active_branch) {
                    b.tip = None;
                }
                return Ok(removed);
            }
        }

        // Update branch tip
        if let Some(b) = self.branches.get_mut(&self.active_branch) {
            b.tip = Some(current_tip);
        }

        Ok(removed)
    }

    /// Rollback N "turns" (user+assistant pairs) from the current branch.
    ///
    /// A turn is counted as a user message followed by any number of
    /// assistant/tool messages until the next user message.
    pub fn rollback_turns(&mut self, n: usize) -> Result<usize, BranchError> {
        if n == 0 {
            return Ok(0);
        }

        let messages = self.current_messages();
        if messages.is_empty() {
            return Ok(0);
        }

        // Count turns from the end
        let mut turns_found = 0;
        let mut messages_to_remove = 0;

        for msg in messages.iter().rev() {
            messages_to_remove += 1;
            if msg.role == crab_core::message::Role::User {
                turns_found += 1;
                if turns_found >= n {
                    break;
                }
            }
        }

        self.rollback(messages_to_remove)
    }

    /// List all branch IDs.
    #[must_use]
    pub fn branch_ids(&self) -> Vec<&BranchId> {
        self.branches.keys().collect()
    }

    /// Get branch metadata.
    #[must_use]
    pub fn get_branch(&self, id: &BranchId) -> Option<&Branch> {
        self.branches.get(id)
    }

    /// Total number of nodes in the tree.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of messages on the current branch path (root to tip).
    #[must_use]
    pub fn current_depth(&self) -> usize {
        self.current_messages().len()
    }

    /// Number of branches.
    #[must_use]
    pub fn branch_count(&self) -> usize {
        self.branches.len()
    }

    /// Get a node by index.
    #[must_use]
    pub fn get_node(&self, index: usize) -> Option<&ConversationNode> {
        self.nodes.get(index)
    }

    /// Delete a branch (cannot delete the main branch or the active branch).
    pub fn delete_branch(&mut self, id: &BranchId) -> Result<(), BranchError> {
        if id.as_str() == MAIN_BRANCH {
            return Err(BranchError::CannotDeleteMain);
        }
        if *id == self.active_branch {
            return Err(BranchError::CannotDeleteActive(id.clone()));
        }
        if self.branches.remove(id).is_none() {
            return Err(BranchError::BranchNotFound(id.clone()));
        }
        Ok(())
    }

    /// Check if the tree is empty (no nodes at all).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get the messages for a given branch (root to tip).
    #[must_use]
    pub fn branch_messages(&self, id: &BranchId) -> Vec<&Message> {
        self.branches
            .get(id)
            .map(|b| self.messages_to_tip(b.tip))
            .unwrap_or_default()
    }
}

impl Default for ConversationTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during branch operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchError {
    /// The specified node index does not exist.
    InvalidNode(usize),
    /// The branch was not found.
    BranchNotFound(BranchId),
    /// Cannot delete the main branch.
    CannotDeleteMain,
    /// Cannot delete the currently active branch.
    CannotDeleteActive(BranchId),
    /// The branch is empty (no messages to fork from).
    EmptyBranch(BranchId),
}

impl std::fmt::Display for BranchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidNode(idx) => write!(f, "invalid node index: {idx}"),
            Self::BranchNotFound(id) => write!(f, "branch not found: {id}"),
            Self::CannotDeleteMain => write!(f, "cannot delete the main branch"),
            Self::CannotDeleteActive(id) => write!(f, "cannot delete the active branch: {id}"),
            Self::EmptyBranch(id) => write!(f, "branch is empty: {id}"),
        }
    }
}

impl std::error::Error for BranchError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::message::Message;

    fn msg_user(text: &str) -> Message {
        Message::user(text)
    }

    fn msg_assistant(text: &str) -> Message {
        Message::assistant(text)
    }

    // ── Basic operations ────────────────────────────────────────────

    #[test]
    fn new_tree_has_main_branch() {
        let tree = ConversationTree::new();
        assert!(tree.is_empty());
        assert_eq!(tree.branch_count(), 1);
        assert_eq!(tree.active_branch().as_str(), "main");
        assert_eq!(tree.current_depth(), 0);
    }

    #[test]
    fn push_messages_on_main() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Hello"));
        tree.push(msg_assistant("Hi there!"));

        assert_eq!(tree.node_count(), 2);
        assert_eq!(tree.current_depth(), 2);

        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text(), "Hello");
        assert_eq!(msgs[1].text(), "Hi there!");
    }

    #[test]
    fn push_builds_parent_child_links() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("A"));
        tree.push(msg_assistant("B"));
        tree.push(msg_user("C"));

        let node0 = tree.get_node(0).unwrap();
        assert!(node0.parent.is_none());
        assert_eq!(node0.children, vec![1]);

        let node1 = tree.get_node(1).unwrap();
        assert_eq!(node1.parent, Some(0));
        assert_eq!(node1.children, vec![2]);

        let node2 = tree.get_node(2).unwrap();
        assert_eq!(node2.parent, Some(1));
        assert!(node2.children.is_empty());
    }

    // ── Branching ───────────────────────────────────────────────────

    #[test]
    fn create_branch_from_node() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("A"));
        tree.push(msg_assistant("B"));

        let branch_id = tree.create_branch(0, Some("Alt path".into())).unwrap();
        assert_eq!(tree.branch_count(), 2);

        let branch = tree.get_branch(&branch_id).unwrap();
        assert_eq!(branch.fork_point, Some(0));
        assert_eq!(branch.tip, Some(0));
        assert_eq!(branch.label.as_deref(), Some("Alt path"));
    }

    #[test]
    fn create_branch_invalid_node() {
        let mut tree = ConversationTree::new();
        let err = tree.create_branch(99, None).unwrap_err();
        assert_eq!(err, BranchError::InvalidNode(99));
    }

    #[test]
    fn fork_and_switch() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        let branch_id = tree.fork_and_switch(0, None).unwrap();
        assert_eq!(tree.active_branch(), &branch_id);

        // Current messages should be just the fork point message
        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text(), "Q1");

        // Push on new branch
        tree.push(msg_assistant("Alternative answer"));
        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].text(), "Alternative answer");
    }

    #[test]
    fn fork_here_creates_branch_at_tip() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        let branch_id = tree.fork_here(Some("exploration".into())).unwrap();
        assert_eq!(tree.active_branch(), &branch_id);

        // Messages should include everything up to the fork point
        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn fork_here_on_empty_branch_fails() {
        let mut tree = ConversationTree::new();
        let err = tree.fork_here(None).unwrap_err();
        assert!(matches!(err, BranchError::EmptyBranch(_)));
    }

    #[test]
    fn divergent_branches_have_independent_histories() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        let main_id = tree.active_branch().clone();

        // Fork from node 0 (Q1)
        let alt_id = tree.fork_and_switch(0, None).unwrap();
        tree.push(msg_assistant("Alt-A1"));
        tree.push(msg_user("Q2-alt"));

        let alt_msgs = tree.current_messages();
        assert_eq!(alt_msgs.len(), 3); // Q1 + Alt-A1 + Q2-alt

        // Switch back to main
        tree.switch_branch(&main_id).unwrap();
        let main_msgs = tree.current_messages();
        assert_eq!(main_msgs.len(), 2); // Q1 + A1

        // Switch to alt again
        tree.switch_branch(&alt_id).unwrap();
        assert_eq!(tree.current_messages().len(), 3);
    }

    // ── Switch ──────────────────────────────────────────────────────

    #[test]
    fn switch_branch_not_found() {
        let mut tree = ConversationTree::new();
        let err = tree
            .switch_branch(&BranchId::new("nonexistent"))
            .unwrap_err();
        assert!(matches!(err, BranchError::BranchNotFound(_)));
    }

    #[test]
    fn switch_to_same_branch() {
        let mut tree = ConversationTree::new();
        let main_id = tree.active_branch().clone();
        tree.switch_branch(&main_id).unwrap();
        assert_eq!(tree.active_branch().as_str(), "main");
    }

    // ── Rollback ────────────────────────────────────────────────────

    #[test]
    fn rollback_removes_messages() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));
        tree.push(msg_user("Q2"));
        tree.push(msg_assistant("A2"));

        let removed = tree.rollback(2).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(tree.current_depth(), 2);

        let msgs = tree.current_messages();
        assert_eq!(msgs[0].text(), "Q1");
        assert_eq!(msgs[1].text(), "A1");
    }

    #[test]
    fn rollback_zero_is_noop() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        let removed = tree.rollback(0).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(tree.current_depth(), 1);
    }

    #[test]
    fn rollback_more_than_available() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        let removed = tree.rollback(100).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(tree.current_depth(), 0);
        assert!(tree.current_tip().is_none());
    }

    #[test]
    fn rollback_on_empty_branch() {
        let mut tree = ConversationTree::new();
        let removed = tree.rollback(5).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn rollback_stops_at_fork_point() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        // Fork from node 1 (A1)
        tree.fork_and_switch(1, None).unwrap();
        tree.push(msg_user("Q2-alt"));
        tree.push(msg_assistant("A2-alt"));

        // Rollback 10 — should stop at fork point (node 1)
        let removed = tree.rollback(10).unwrap();
        assert_eq!(removed, 2); // only Q2-alt and A2-alt removed

        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 2); // Q1 + A1 (fork point)
    }

    #[test]
    fn rollback_updates_parent_children() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        tree.rollback(1).unwrap();

        // Node 0 should no longer list node 1 as a child
        let node0 = tree.get_node(0).unwrap();
        assert!(!node0.children.contains(&1));
    }

    // ── Rollback turns ──────────────────────────────────────────────

    #[test]
    fn rollback_turns_removes_full_turn() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));
        tree.push(msg_user("Q2"));
        tree.push(msg_assistant("A2"));

        let removed = tree.rollback_turns(1).unwrap();
        assert_eq!(removed, 2); // Q2 + A2

        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].text(), "A1");
    }

    #[test]
    fn rollback_turns_multiple() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));
        tree.push(msg_user("Q2"));
        tree.push(msg_assistant("A2"));
        tree.push(msg_user("Q3"));
        tree.push(msg_assistant("A3"));

        let removed = tree.rollback_turns(2).unwrap();
        assert_eq!(removed, 4); // Q2+A2+Q3+A3

        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text(), "Q1");
    }

    #[test]
    fn rollback_turns_zero_is_noop() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        let removed = tree.rollback_turns(0).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(tree.current_depth(), 2);
    }

    #[test]
    fn rollback_turns_on_empty() {
        let mut tree = ConversationTree::new();
        let removed = tree.rollback_turns(1).unwrap();
        assert_eq!(removed, 0);
    }

    // ── Delete branch ───────────────────────────────────────────────

    #[test]
    fn delete_branch() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        let branch_id = tree.create_branch(0, None).unwrap();
        assert_eq!(tree.branch_count(), 2);

        tree.delete_branch(&branch_id).unwrap();
        assert_eq!(tree.branch_count(), 1);
    }

    #[test]
    fn cannot_delete_main() {
        let mut tree = ConversationTree::new();
        let err = tree.delete_branch(&BranchId::new("main")).unwrap_err();
        assert_eq!(err, BranchError::CannotDeleteMain);
    }

    #[test]
    fn cannot_delete_active_branch() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        let branch_id = tree.fork_and_switch(0, None).unwrap();
        let err = tree.delete_branch(&branch_id).unwrap_err();
        assert!(matches!(err, BranchError::CannotDeleteActive(_)));
    }

    #[test]
    fn delete_nonexistent_branch() {
        let mut tree = ConversationTree::new();
        let err = tree.delete_branch(&BranchId::new("ghost")).unwrap_err();
        assert!(matches!(err, BranchError::BranchNotFound(_)));
    }

    // ── Branch messages ─────────────────────────────────────────────

    #[test]
    fn branch_messages_returns_path() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));

        let main_id = BranchId::new("main");
        let msgs = tree.branch_messages(&main_id);
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn branch_messages_nonexistent_returns_empty() {
        let tree = ConversationTree::new();
        let msgs = tree.branch_messages(&BranchId::new("nope"));
        assert!(msgs.is_empty());
    }

    // ── Branch IDs listing ──────────────────────────────────────────

    #[test]
    fn branch_ids_lists_all() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.create_branch(0, None).unwrap();
        tree.create_branch(0, None).unwrap();

        let ids = tree.branch_ids();
        assert_eq!(ids.len(), 3);
    }

    // ── Default impl ────────────────────────────────────────────────

    #[test]
    fn default_is_new() {
        let tree = ConversationTree::default();
        assert!(tree.is_empty());
        assert_eq!(tree.branch_count(), 1);
    }

    // ── Multi-branch divergence scenario ────────────────────────────

    #[test]
    fn multiple_branches_from_same_node() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1")); // node 1

        let main_id = tree.active_branch().clone();

        // Branch A from node 0
        let a_id = tree.fork_and_switch(0, Some("branch-a".into())).unwrap();
        tree.push(msg_assistant("A1-alt-a"));

        // Switch back to main, branch B also from node 0
        tree.switch_branch(&main_id).unwrap();
        let b_id = tree.fork_and_switch(0, Some("branch-b".into())).unwrap();
        tree.push(msg_assistant("A1-alt-b"));

        // Node 0 should have 3 children: node 1 (main), branch-a tip, branch-b tip
        let node0 = tree.get_node(0).unwrap();
        assert_eq!(node0.children.len(), 3);

        // Verify independent paths
        tree.switch_branch(&a_id).unwrap();
        assert_eq!(tree.current_messages().last().unwrap().text(), "A1-alt-a");

        tree.switch_branch(&b_id).unwrap();
        assert_eq!(tree.current_messages().last().unwrap().text(), "A1-alt-b");

        tree.switch_branch(&main_id).unwrap();
        assert_eq!(tree.current_messages().last().unwrap().text(), "A1");
    }

    // ── BranchError Display ─────────────────────────────────────────

    #[test]
    fn branch_error_display() {
        let e = BranchError::InvalidNode(42);
        assert_eq!(e.to_string(), "invalid node index: 42");

        let e = BranchError::CannotDeleteMain;
        assert_eq!(e.to_string(), "cannot delete the main branch");

        let e = BranchError::BranchNotFound(BranchId::new("x"));
        assert_eq!(e.to_string(), "branch not found: x");

        let e = BranchError::CannotDeleteActive(BranchId::new("y"));
        assert_eq!(e.to_string(), "cannot delete the active branch: y");

        let e = BranchError::EmptyBranch(BranchId::new("z"));
        assert_eq!(e.to_string(), "branch is empty: z");
    }

    // ── BranchId Display ────────────────────────────────────────────

    #[test]
    fn branch_id_display() {
        let id = BranchId::new("test-branch");
        assert_eq!(id.to_string(), "test-branch");
        assert_eq!(id.as_str(), "test-branch");
    }

    // ── Node access ─────────────────────────────────────────────────

    #[test]
    fn get_node_out_of_bounds() {
        let tree = ConversationTree::new();
        assert!(tree.get_node(0).is_none());
        assert!(tree.get_node(100).is_none());
    }

    // ── Rollback then push continues correctly ──────────────────────

    #[test]
    fn rollback_then_push_continues_from_new_tip() {
        let mut tree = ConversationTree::new();
        tree.push(msg_user("Q1"));
        tree.push(msg_assistant("A1"));
        tree.push(msg_user("Q2"));
        tree.push(msg_assistant("A2"));

        tree.rollback(2).unwrap();

        // Push new messages after rollback
        tree.push(msg_user("Q2-new"));
        tree.push(msg_assistant("A2-new"));

        let msgs = tree.current_messages();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2].text(), "Q2-new");
        assert_eq!(msgs[3].text(), "A2-new");
    }
}
