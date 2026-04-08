//! Official MCP server registry: well-known servers with default configs.
//!
//! Provides a catalog of officially supported MCP servers (e.g. Playwright,
//! filesystem, GitHub) with their default transport configurations. Users
//! can reference servers by name instead of specifying full config.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Types ─────────────────────────────────────────────────────────────

/// An entry in the official MCP server registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Short canonical name (e.g. "playwright", "filesystem").
    pub name: String,
    /// Human-readable description of the server's capabilities.
    pub description: String,
    /// Default MCP transport configuration for this server.
    ///
    /// Follows the `McpServerConfig` JSON schema so it can be merged
    /// directly into the user's settings.
    pub default_config: Value,
}

// ── Lookup ────────────────────────────────────────────────────────────

/// Look up an official server by its canonical name.
///
/// Returns `None` if the name is not in the registry.
#[must_use]
pub fn lookup_server(_name: &str) -> Option<RegistryEntry> {
    todo!("lookup_server: search the built-in registry for the given name")
}

/// List all officially registered MCP servers.
///
/// Returns entries sorted alphabetically by name.
#[must_use]
pub fn list_official_servers() -> Vec<RegistryEntry> {
    todo!("list_official_servers: return the full catalog of known servers")
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_entry_serde_roundtrip() {
        let entry = RegistryEntry {
            name: "playwright".into(),
            description: "Browser automation via Playwright".into(),
            default_config: serde_json::json!({
                "command": "npx",
                "args": ["@anthropic-ai/mcp-playwright"]
            }),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: RegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "playwright");
        assert!(!parsed.description.is_empty());
    }
}
