//! Task assignment strategies for distributing work across agents.
//!
//! Strategies decide which agent should receive a new task based on
//! different criteria (round-robin, capability matching, etc.).

use crate::team::{Capability, TeamMember};

/// A strategy for choosing which team member should handle a task.
pub trait AssignmentStrategy: Send + Sync {
    /// Select the best member from `candidates` for the given task.
    ///
    /// `required_capability` is an optional hint about what the task needs.
    /// Returns `None` if no suitable candidate is found.
    fn select<'a>(
        &mut self,
        candidates: &'a [TeamMember],
        required_capability: Option<&Capability>,
    ) -> Option<&'a TeamMember>;

    /// Name of this strategy (for logging/display).
    fn name(&self) -> &str;
}

/// Round-robin assignment — cycles through candidates in order.
///
/// Ignores capabilities; assigns purely by rotation. Simple and fair
/// for homogeneous worker pools.
pub struct RoundRobin {
    next_index: usize,
}

impl RoundRobin {
    #[must_use]
    pub fn new() -> Self {
        Self { next_index: 0 }
    }
}

impl Default for RoundRobin {
    fn default() -> Self {
        Self::new()
    }
}

impl AssignmentStrategy for RoundRobin {
    fn select<'a>(
        &mut self,
        candidates: &'a [TeamMember],
        _required_capability: Option<&Capability>,
    ) -> Option<&'a TeamMember> {
        if candidates.is_empty() {
            return None;
        }
        let idx = self.next_index % candidates.len();
        self.next_index = idx + 1;
        Some(&candidates[idx])
    }

    fn name(&self) -> &'static str {
        "round-robin"
    }
}

/// Capability-based assignment — picks the first member that has the
/// required capability. Falls back to the first candidate if no
/// capability is specified or no match is found.
pub struct CapabilityBased;

impl CapabilityBased {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CapabilityBased {
    fn default() -> Self {
        Self::new()
    }
}

impl AssignmentStrategy for CapabilityBased {
    fn select<'a>(
        &mut self,
        candidates: &'a [TeamMember],
        required_capability: Option<&Capability>,
    ) -> Option<&'a TeamMember> {
        if candidates.is_empty() {
            return None;
        }

        if let Some(cap) = required_capability {
            // Find the first candidate with the required capability
            if let Some(member) = candidates.iter().find(|m| m.has_capability(cap)) {
                return Some(member);
            }
        }

        // Fallback: first candidate
        Some(&candidates[0])
    }

    fn name(&self) -> &'static str {
        "capability-based"
    }
}

/// Least-loaded assignment — picks the member with the fewest active tasks.
///
/// Requires external load information passed as a closure.
pub struct LeastLoaded<F>
where
    F: Fn(&str) -> usize + Send + Sync,
{
    load_fn: F,
}

impl<F> LeastLoaded<F>
where
    F: Fn(&str) -> usize + Send + Sync,
{
    /// Create a new least-loaded strategy.
    ///
    /// `load_fn` takes an agent name and returns its current task count.
    pub fn new(load_fn: F) -> Self {
        Self { load_fn }
    }
}

impl<F> AssignmentStrategy for LeastLoaded<F>
where
    F: Fn(&str) -> usize + Send + Sync,
{
    fn select<'a>(
        &mut self,
        candidates: &'a [TeamMember],
        required_capability: Option<&Capability>,
    ) -> Option<&'a TeamMember> {
        if candidates.is_empty() {
            return None;
        }

        let filtered: Vec<&TeamMember> = required_capability.map_or_else(
            || candidates.iter().collect(),
            |cap| {
                candidates
                    .iter()
                    .filter(|m| m.has_capability(cap))
                    .collect()
            },
        );

        if filtered.is_empty() {
            // Fallback to all candidates if no capability match
            return candidates.iter().min_by_key(|m| (self.load_fn)(&m.name));
        }

        filtered.into_iter().min_by_key(|m| (self.load_fn)(&m.name))
    }

    fn name(&self) -> &'static str {
        "least-loaded"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::TeamMember;
    use std::collections::HashMap;

    fn make_members() -> Vec<TeamMember> {
        let mut alice = TeamMember::new("a1", "Alice", "claude-3");
        alice.add_capability(Capability::new("code_review"));
        alice.add_capability(Capability::new("planning"));

        let mut bob = TeamMember::new("a2", "Bob", "gpt-4o");
        bob.add_capability(Capability::new("testing"));
        bob.add_capability(Capability::new("code_review"));

        let mut charlie = TeamMember::new("a3", "Charlie", "claude-3");
        charlie.add_capability(Capability::new("frontend"));

        vec![alice, bob, charlie]
    }

    // ─── RoundRobin ───

    #[test]
    fn round_robin_cycles() {
        let members = make_members();
        let mut rr = RoundRobin::new();

        assert_eq!(rr.select(&members, None).unwrap().name, "Alice");
        assert_eq!(rr.select(&members, None).unwrap().name, "Bob");
        assert_eq!(rr.select(&members, None).unwrap().name, "Charlie");
        // Wraps around
        assert_eq!(rr.select(&members, None).unwrap().name, "Alice");
    }

    #[test]
    fn round_robin_empty() {
        let mut rr = RoundRobin::new();
        assert!(rr.select(&[], None).is_none());
    }

    #[test]
    fn round_robin_single() {
        let members = vec![TeamMember::new("a1", "Alice", "model")];
        let mut rr = RoundRobin::new();
        assert_eq!(rr.select(&members, None).unwrap().name, "Alice");
        assert_eq!(rr.select(&members, None).unwrap().name, "Alice");
    }

    #[test]
    fn round_robin_ignores_capability() {
        let members = make_members();
        let mut rr = RoundRobin::new();
        let cap = Capability::new("frontend");
        // Even with a capability hint, round-robin ignores it
        assert_eq!(rr.select(&members, Some(&cap)).unwrap().name, "Alice");
    }

    #[test]
    fn round_robin_name() {
        let rr = RoundRobin::new();
        assert_eq!(rr.name(), "round-robin");
    }

    #[test]
    fn round_robin_default() {
        let mut rr = RoundRobin::default();
        let members = make_members();
        assert_eq!(rr.select(&members, None).unwrap().name, "Alice");
    }

    // ─── CapabilityBased ───

    #[test]
    fn capability_based_finds_match() {
        let members = make_members();
        let mut cb = CapabilityBased::new();

        let cap = Capability::new("testing");
        let selected = cb.select(&members, Some(&cap)).unwrap();
        assert_eq!(selected.name, "Bob"); // only Bob has "testing"
    }

    #[test]
    fn capability_based_first_match() {
        let members = make_members();
        let mut cb = CapabilityBased::new();

        let cap = Capability::new("code_review");
        let selected = cb.select(&members, Some(&cap)).unwrap();
        assert_eq!(selected.name, "Alice"); // Alice is first with "code_review"
    }

    #[test]
    fn capability_based_no_match_falls_back() {
        let members = make_members();
        let mut cb = CapabilityBased::new();

        let cap = Capability::new("devops");
        let selected = cb.select(&members, Some(&cap)).unwrap();
        assert_eq!(selected.name, "Alice"); // fallback to first
    }

    #[test]
    fn capability_based_no_capability_falls_back() {
        let members = make_members();
        let mut cb = CapabilityBased::new();

        let selected = cb.select(&members, None).unwrap();
        assert_eq!(selected.name, "Alice"); // fallback to first
    }

    #[test]
    fn capability_based_empty() {
        let mut cb = CapabilityBased::new();
        assert!(cb.select(&[], Some(&Capability::new("x"))).is_none());
    }

    #[test]
    fn capability_based_name() {
        let cb = CapabilityBased::new();
        assert_eq!(cb.name(), "capability-based");
    }

    #[test]
    fn capability_based_default() {
        let mut cb = CapabilityBased::default();
        let members = make_members();
        assert!(cb.select(&members, None).is_some());
    }

    // ─── LeastLoaded ───

    #[test]
    fn least_loaded_picks_lightest() {
        let members = make_members();
        let loads: HashMap<String, usize> = [
            ("Alice".into(), 3),
            ("Bob".into(), 1),
            ("Charlie".into(), 5),
        ]
        .into_iter()
        .collect();

        let mut ll = LeastLoaded::new(move |name: &str| *loads.get(name).unwrap_or(&0));

        let selected = ll.select(&members, None).unwrap();
        assert_eq!(selected.name, "Bob"); // lowest load
    }

    #[test]
    fn least_loaded_with_capability_filter() {
        let members = make_members();
        let loads: HashMap<String, usize> = [
            ("Alice".into(), 5),
            ("Bob".into(), 1),
            ("Charlie".into(), 0),
        ]
        .into_iter()
        .collect();

        let mut ll = LeastLoaded::new(move |name: &str| *loads.get(name).unwrap_or(&0));

        // Require "code_review" — Alice(5) and Bob(1) have it, Bob is lighter
        let cap = Capability::new("code_review");
        let selected = ll.select(&members, Some(&cap)).unwrap();
        assert_eq!(selected.name, "Bob");
    }

    #[test]
    fn least_loaded_no_capability_match_falls_back() {
        let members = make_members();
        let loads: HashMap<String, usize> = [
            ("Alice".into(), 2),
            ("Bob".into(), 5),
            ("Charlie".into(), 1),
        ]
        .into_iter()
        .collect();

        let mut ll = LeastLoaded::new(move |name: &str| *loads.get(name).unwrap_or(&0));

        // "devops" — nobody has it, fall back to lightest overall
        let cap = Capability::new("devops");
        let selected = ll.select(&members, Some(&cap)).unwrap();
        assert_eq!(selected.name, "Charlie");
    }

    #[test]
    fn least_loaded_empty() {
        let mut ll = LeastLoaded::new(|_: &str| 0);
        assert!(ll.select(&[], None).is_none());
    }

    #[test]
    fn least_loaded_name() {
        let ll = LeastLoaded::new(|_: &str| 0);
        assert_eq!(ll.name(), "least-loaded");
    }

    // ─── Trait object compatibility ───

    #[test]
    fn strategy_as_trait_object() {
        let members = make_members();
        let mut strategy: Box<dyn AssignmentStrategy> = Box::new(RoundRobin::new());
        assert_eq!(strategy.name(), "round-robin");
        assert!(strategy.select(&members, None).is_some());

        strategy = Box::new(CapabilityBased::new());
        assert_eq!(strategy.name(), "capability-based");
        assert!(strategy.select(&members, None).is_some());
    }
}
