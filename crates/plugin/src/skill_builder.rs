//! Fluent API for constructing skills + MCP skill loading.
//!
//! Provides a builder pattern for creating [`Skill`](super::skill::Skill) instances
//! programmatically, and a helper to convert MCP server tool lists into native skills.
//!
//! Maps to CCB `skills/mcpSkills.ts` + `skills/mcpSkillBuilders.ts`.

use super::skill::Skill;

// ─── Skill builder ─────────────────────────────────────────────────────

/// Fluent builder for constructing [`Skill`] instances.
///
/// # Example
///
/// ```no_run
/// use crab_plugin::skill_builder::SkillBuilder;
///
/// let skill = SkillBuilder::new("commit")
///     .description("Create a git commit with a good message")
///     .content("You are a commit helper. ...")
///     .trigger("/commit")
///     .build();
/// ```
pub struct SkillBuilder {
    name: String,
    description: Option<String>,
    content: Option<String>,
    trigger_patterns: Vec<String>,
}

impl SkillBuilder {
    /// Start building a skill with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            content: None,
            trigger_patterns: Vec::new(),
        }
    }

    /// Set the human-readable description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the skill's prompt content (markdown body).
    #[must_use]
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Add a trigger pattern.
    ///
    /// Patterns starting with `/` become `SkillTrigger::Command`,
    /// other patterns become `SkillTrigger::Pattern`.
    #[must_use]
    pub fn trigger(mut self, pattern: impl Into<String>) -> Self {
        self.trigger_patterns.push(pattern.into());
        self
    }

    /// Consume the builder and produce a [`Skill`].
    ///
    /// # Errors
    ///
    /// Returns `Err` if the skill name is empty or if no content was provided.
    pub fn build(self) -> Result<Skill, String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err("skill name must not be empty".into());
        }

        let content = self
            .content
            .filter(|c| !c.trim().is_empty())
            .ok_or("skill content must not be empty")?;

        let trigger = if let Some(first) = self.trigger_patterns.first() {
            if let Some(cmd) = first.strip_prefix('/') {
                super::skill::SkillTrigger::Command {
                    name: cmd.to_string(),
                }
            } else {
                super::skill::SkillTrigger::Pattern {
                    regex: first.clone(),
                }
            }
        } else {
            super::skill::SkillTrigger::Manual
        };

        Ok(Skill {
            name,
            description: self.description.unwrap_or_default(),
            trigger,
            content,
            source_path: None,
        })
    }
}

// ─── MCP skill loading ────────────────────────────────────────────────

/// Convert an MCP server's tool list into native [`Skill`] instances.
///
/// Each MCP tool becomes a skill named `<server_name>:<tool_name>` with
/// the tool's description as the skill description and a `Manual` trigger.
///
/// # Arguments
///
/// * `server_name` — Name of the MCP server (used as skill name prefix).
/// * `tools` — Array of MCP tool definition JSON objects (each with `name`,
///   `description`, etc.).
pub fn load_mcp_skills(server_name: &str, tools: &[serde_json::Value]) -> Vec<Skill> {
    tools
        .iter()
        .filter_map(|tool| {
            let tool_name = tool.get("name")?.as_str()?;
            let desc = tool
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let full_name = format!("{server_name}:{tool_name}");
            SkillBuilder::new(full_name)
                .description(desc)
                .content(desc)
                .build()
                .ok()
        })
        .collect()
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_chain_compiles() {
        let _builder = SkillBuilder::new("test")
            .description("A test skill")
            .content("prompt content")
            .trigger("/test")
            .trigger("test.*pattern");
        // build() would panic with todo!(), just verify the chain compiles
    }

    #[test]
    fn builder_new_sets_name() {
        let builder = SkillBuilder::new("my-skill");
        assert_eq!(builder.name, "my-skill");
        assert!(builder.description.is_none());
        assert!(builder.content.is_none());
        assert!(builder.trigger_patterns.is_empty());
    }

    #[test]
    fn builder_accumulates_triggers() {
        let builder = SkillBuilder::new("x").trigger("/cmd").trigger("pattern.*");
        assert_eq!(builder.trigger_patterns.len(), 2);
        assert_eq!(builder.trigger_patterns[0], "/cmd");
        assert_eq!(builder.trigger_patterns[1], "pattern.*");
    }

    #[test]
    fn build_command_trigger() {
        let skill = SkillBuilder::new("test")
            .content("prompt")
            .trigger("/test")
            .build()
            .unwrap();
        assert_eq!(skill.name, "test");
        assert!(matches!(
            skill.trigger,
            super::super::skill::SkillTrigger::Command { .. }
        ));
    }

    #[test]
    fn build_manual_trigger() {
        let skill = SkillBuilder::new("test").content("prompt").build().unwrap();
        assert!(matches!(
            skill.trigger,
            super::super::skill::SkillTrigger::Manual
        ));
    }

    #[test]
    fn build_empty_name_fails() {
        let result = SkillBuilder::new("").content("x").build();
        assert!(result.is_err());
    }

    #[test]
    fn build_no_content_fails() {
        let result = SkillBuilder::new("test").build();
        assert!(result.is_err());
    }

    #[test]
    fn load_mcp_skills_basic() {
        let tools = vec![serde_json::json!({
            "name": "read_file",
            "description": "Read a file from disk"
        })];
        let skills = load_mcp_skills("filesystem", &tools);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "filesystem:read_file");
    }

    #[test]
    fn load_mcp_skills_skips_invalid() {
        let tools = vec![
            serde_json::json!({"name": "good", "description": "works"}),
            serde_json::json!({"invalid": "no name field"}),
        ];
        let skills = load_mcp_skills("srv", &tools);
        assert_eq!(skills.len(), 1);
    }
}
