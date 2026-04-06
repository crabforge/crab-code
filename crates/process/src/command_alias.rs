//! Command alias registry — shorthand names that expand to full commands.
//!
//! Provides [`AliasRegistry`] for registering, resolving, and persisting
//! command aliases. Includes built-in aliases for common development tools.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Types ───────────────────────────────────────────────────────────

/// A single command alias definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandAlias {
    /// Short name (e.g. `"build"`).
    pub name: String,
    /// Full command expansion (e.g. `"cargo build"`).
    pub expansion: String,
    /// Optional human-readable description.
    pub description: String,
}

/// Result of resolving user input against the alias registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCommand {
    /// The input matched an alias and was expanded.
    Expanded {
        /// The original alias name.
        alias: String,
        /// The expanded command string.
        command: String,
        /// Any extra arguments appended after the alias.
        extra_args: Vec<String>,
    },
    /// No alias matched; the input is used as-is.
    Passthrough {
        /// The original command.
        command: String,
        /// Arguments.
        args: Vec<String>,
    },
}

impl ResolvedCommand {
    /// The full command line as a single string.
    #[must_use]
    pub fn command_line(&self) -> String {
        match self {
            Self::Expanded {
                command,
                extra_args,
                ..
            } => {
                if extra_args.is_empty() {
                    command.clone()
                } else {
                    format!("{command} {}", extra_args.join(" "))
                }
            }
            Self::Passthrough { command, args } => {
                if args.is_empty() {
                    command.clone()
                } else {
                    format!("{command} {}", args.join(" "))
                }
            }
        }
    }
}

// ── Registry ────────────────────────────────────────────────────────

/// Manages command aliases (built-in + user-defined).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasRegistry {
    aliases: BTreeMap<String, CommandAlias>,
}

impl AliasRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            aliases: BTreeMap::new(),
        }
    }

    /// Create a registry pre-loaded with built-in aliases.
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        for alias in builtin_aliases() {
            reg.aliases.insert(alias.name.clone(), alias);
        }
        reg
    }

    /// Register or update an alias.
    pub fn register(&mut self, alias: CommandAlias) {
        self.aliases.insert(alias.name.clone(), alias);
    }

    /// Remove an alias by name. Returns the removed alias if it existed.
    pub fn remove(&mut self, name: &str) -> Option<CommandAlias> {
        self.aliases.remove(name)
    }

    /// Look up an alias by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CommandAlias> {
        self.aliases.get(name)
    }

    /// List all registered aliases, sorted by name.
    #[must_use]
    pub fn list(&self) -> Vec<&CommandAlias> {
        self.aliases.values().collect()
    }

    /// Number of registered aliases.
    #[must_use]
    pub fn len(&self) -> usize {
        self.aliases.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }

    /// Resolve user input, expanding aliases if matched.
    ///
    /// The first whitespace-delimited token is checked against the registry.
    /// If it matches, the alias expansion is used; remaining tokens become
    /// extra arguments.
    #[must_use]
    pub fn resolve(&self, input: &str) -> ResolvedCommand {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return ResolvedCommand::Passthrough {
                command: String::new(),
                args: Vec::new(),
            };
        }

        let first = parts[0];
        let rest: Vec<String> = parts[1..].iter().map(|s| (*s).to_owned()).collect();

        if let Some(alias) = self.aliases.get(first) {
            ResolvedCommand::Expanded {
                alias: alias.name.clone(),
                command: alias.expansion.clone(),
                extra_args: rest,
            }
        } else {
            ResolvedCommand::Passthrough {
                command: first.to_owned(),
                args: rest,
            }
        }
    }

    // ── Persistence ─────────────────────────────────────────

    /// Save aliases to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> crab_common::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| crab_common::Error::Other(format!("serialize error: {e}")))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load aliases from a JSON file. Returns an empty registry if the file
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load(path: &Path) -> crab_common::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let data = std::fs::read_to_string(path)?;
        serde_json::from_str(&data)
            .map_err(|e| crab_common::Error::Other(format!("parse error: {e}")))
    }
}

impl Default for AliasRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

// ── Built-in aliases ────────────────────────────────────────────────

/// Default built-in aliases for common development commands.
#[must_use]
pub fn builtin_aliases() -> Vec<CommandAlias> {
    vec![
        CommandAlias {
            name: "build".to_owned(),
            expansion: "cargo build".to_owned(),
            description: "Build the project with cargo".to_owned(),
        },
        CommandAlias {
            name: "test".to_owned(),
            expansion: "cargo test".to_owned(),
            description: "Run tests with cargo".to_owned(),
        },
        CommandAlias {
            name: "lint".to_owned(),
            expansion: "cargo clippy".to_owned(),
            description: "Run clippy linter".to_owned(),
        },
        CommandAlias {
            name: "fmt".to_owned(),
            expansion: "cargo fmt".to_owned(),
            description: "Format code with rustfmt".to_owned(),
        },
        CommandAlias {
            name: "check".to_owned(),
            expansion: "cargo check".to_owned(),
            description: "Check compilation without building".to_owned(),
        },
        CommandAlias {
            name: "run".to_owned(),
            expansion: "cargo run".to_owned(),
            description: "Build and run the project".to_owned(),
        },
    ]
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let reg = AliasRegistry::new();
        assert!(reg.is_empty());
    }

    #[test]
    fn with_builtins_has_aliases() {
        let reg = AliasRegistry::with_builtins();
        assert!(reg.len() >= 6);
        assert!(reg.get("build").is_some());
        assert!(reg.get("test").is_some());
        assert!(reg.get("lint").is_some());
    }

    #[test]
    fn register_and_get() {
        let mut reg = AliasRegistry::new();
        reg.register(CommandAlias {
            name: "hi".to_owned(),
            expansion: "echo hello".to_owned(),
            description: "Say hello".to_owned(),
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("hi").unwrap().expansion, "echo hello");
    }

    #[test]
    fn register_overwrites() {
        let mut reg = AliasRegistry::new();
        reg.register(CommandAlias {
            name: "x".to_owned(),
            expansion: "old".to_owned(),
            description: String::new(),
        });
        reg.register(CommandAlias {
            name: "x".to_owned(),
            expansion: "new".to_owned(),
            description: String::new(),
        });
        assert_eq!(reg.get("x").unwrap().expansion, "new");
    }

    #[test]
    fn remove_alias() {
        let mut reg = AliasRegistry::with_builtins();
        let removed = reg.remove("build");
        assert!(removed.is_some());
        assert!(reg.get("build").is_none());
    }

    #[test]
    fn remove_nonexistent() {
        let mut reg = AliasRegistry::new();
        assert!(reg.remove("nope").is_none());
    }

    #[test]
    fn list_sorted() {
        let mut reg = AliasRegistry::new();
        reg.register(CommandAlias {
            name: "z".to_owned(),
            expansion: "z".to_owned(),
            description: String::new(),
        });
        reg.register(CommandAlias {
            name: "a".to_owned(),
            expansion: "a".to_owned(),
            description: String::new(),
        });
        let list = reg.list();
        assert_eq!(list[0].name, "a");
        assert_eq!(list[1].name, "z");
    }

    // ── Resolve ─────────────────────────────────────────────

    #[test]
    fn resolve_alias() {
        let reg = AliasRegistry::with_builtins();
        let res = reg.resolve("build --release");
        match res {
            ResolvedCommand::Expanded {
                alias,
                command,
                extra_args,
            } => {
                assert_eq!(alias, "build");
                assert_eq!(command, "cargo build");
                assert_eq!(extra_args, vec!["--release"]);
            }
            _ => panic!("expected Expanded"),
        }
    }

    #[test]
    fn resolve_passthrough() {
        let reg = AliasRegistry::new();
        let res = reg.resolve("git status");
        match res {
            ResolvedCommand::Passthrough { command, args } => {
                assert_eq!(command, "git");
                assert_eq!(args, vec!["status"]);
            }
            _ => panic!("expected Passthrough"),
        }
    }

    #[test]
    fn resolve_empty_input() {
        let reg = AliasRegistry::new();
        let res = reg.resolve("");
        assert!(matches!(res, ResolvedCommand::Passthrough { .. }));
    }

    #[test]
    fn resolve_no_extra_args() {
        let reg = AliasRegistry::with_builtins();
        let res = reg.resolve("lint");
        match res {
            ResolvedCommand::Expanded { extra_args, .. } => {
                assert!(extra_args.is_empty());
            }
            _ => panic!("expected Expanded"),
        }
    }

    #[test]
    fn command_line_expanded() {
        let res = ResolvedCommand::Expanded {
            alias: "build".to_owned(),
            command: "cargo build".to_owned(),
            extra_args: vec!["--release".to_owned()],
        };
        assert_eq!(res.command_line(), "cargo build --release");
    }

    #[test]
    fn command_line_passthrough() {
        let res = ResolvedCommand::Passthrough {
            command: "git".to_owned(),
            args: vec!["status".to_owned()],
        };
        assert_eq!(res.command_line(), "git status");
    }

    #[test]
    fn command_line_no_args() {
        let res = ResolvedCommand::Passthrough {
            command: "ls".to_owned(),
            args: Vec::new(),
        };
        assert_eq!(res.command_line(), "ls");
    }

    // ── Persistence ─────────────────────────────────────────

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aliases.json");

        let mut reg = AliasRegistry::new();
        reg.register(CommandAlias {
            name: "deploy".to_owned(),
            expansion: "kubectl apply -f".to_owned(),
            description: "Deploy to k8s".to_owned(),
        });
        reg.save(&path).unwrap();

        let loaded = AliasRegistry::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("deploy").unwrap().expansion, "kubectl apply -f");
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let loaded = AliasRegistry::load(Path::new("/nonexistent/aliases.json")).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn default_has_builtins() {
        let reg = AliasRegistry::default();
        assert!(reg.get("build").is_some());
    }

    #[test]
    fn builtin_aliases_nonempty() {
        let builtins = builtin_aliases();
        assert!(builtins.len() >= 6);
        for alias in &builtins {
            assert!(!alias.name.is_empty());
            assert!(!alias.expansion.is_empty());
        }
    }

    #[test]
    fn alias_serde_roundtrip() {
        let alias = CommandAlias {
            name: "test".to_owned(),
            expansion: "cargo test".to_owned(),
            description: "Run tests".to_owned(),
        };
        let json = serde_json::to_string(&alias).unwrap();
        let back: CommandAlias = serde_json::from_str(&json).unwrap();
        assert_eq!(alias, back);
    }
}
