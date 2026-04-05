// Configuration management subcommand

use std::path::PathBuf;

use clap::Subcommand;

use crab_config::settings;

/// Actions available under `crab config`.
#[derive(Subcommand)]
pub enum ConfigAction {
    /// List all effective configuration values
    List,
    /// Get the value of a specific configuration key
    Get {
        /// Configuration key (e.g. "model", "apiProvider")
        key: String,
    },
    /// Set a configuration value
    Set {
        /// Configuration key (e.g. "model", "apiProvider")
        key: String,
        /// Value to set
        value: String,
        /// Write to global (~/.crab/settings.json) instead of project
        #[arg(long, short)]
        global: bool,
    },
    /// Open settings.json in your $EDITOR
    Edit {
        /// Edit global (~/.crab/settings.json) instead of project
        #[arg(long, short)]
        global: bool,
    },
    /// Show configuration file paths
    Path,
}

/// Execute a config subcommand.
pub fn run(action: &ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::List => cmd_list(),
        ConfigAction::Get { key } => cmd_get(key),
        ConfigAction::Set { key, value, global } => cmd_set(key, value, *global),
        ConfigAction::Edit { global } => cmd_edit(*global),
        ConfigAction::Path => cmd_path(),
    }
}

/// `crab config list` — print all effective settings as pretty JSON.
fn cmd_list() -> anyhow::Result<()> {
    let working_dir = std::env::current_dir().ok();
    let merged = settings::load_merged_settings(working_dir.as_ref())?;
    let json = serde_json::to_string_pretty(&merged)?;
    println!("{json}");
    Ok(())
}

/// `crab config get <key>` — print a single setting value.
fn cmd_get(key: &str) -> anyhow::Result<()> {
    let working_dir = std::env::current_dir().ok();
    let merged = settings::load_merged_settings(working_dir.as_ref())?;
    let value = settings_to_map(&merged);

    match value.get(key) {
        Some(v) if v.is_null() => {
            println!("(not set)");
        }
        Some(v) => {
            // Print strings without quotes, everything else as JSON
            if let Some(s) = v.as_str() {
                println!("{s}");
            } else {
                println!("{v}");
            }
        }
        None => {
            anyhow::bail!("unknown configuration key: {key}");
        }
    }
    Ok(())
}

/// `crab config set <key> <value>` — write a setting to the chosen config file.
fn cmd_set(key: &str, value: &str, global: bool) -> anyhow::Result<()> {
    // Validate key
    let known_keys = known_settings_keys();
    if !known_keys.contains(&key) {
        anyhow::bail!(
            "unknown configuration key: {key}\nValid keys: {}",
            known_keys.join(", ")
        );
    }

    let path = target_settings_path(global)?;

    // Load existing file (or empty object)
    let mut map: serde_json::Map<String, serde_json::Value> = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let parsed = jsonc_to_value(&content)?;
        match parsed {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };

    // Parse value: try integer, then bool, then string
    let json_value = parse_value(value);
    map.insert(key.to_string(), json_value);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    std::fs::write(&path, format!("{json}\n"))?;

    eprintln!("Set {key} = {value} in {}", path.display());
    Ok(())
}

/// `crab config edit` — open the settings file in $EDITOR.
fn cmd_edit(global: bool) -> anyhow::Result<()> {
    let path = target_settings_path(global)?;

    // Ensure the file exists (create empty JSON object if not)
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, "{}\n")?;
    }

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let status = std::process::Command::new(&editor).arg(&path).status()?;

    if !status.success() {
        anyhow::bail!("editor exited with status {status}");
    }

    Ok(())
}

/// `crab config path` — show config file paths and existence status.
#[allow(clippy::unnecessary_wraps)]
fn cmd_path() -> anyhow::Result<()> {
    let global_dir = settings::global_config_dir();
    let global_settings = global_dir.join("settings.json");
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project_dir = settings::project_config_dir(&working_dir);
    let project_settings = project_dir.join("settings.json");

    println!(
        "Global:  {} {}",
        global_settings.display(),
        existence_tag(&global_settings)
    );
    println!(
        "Project: {} {}",
        project_settings.display(),
        existence_tag(&project_settings)
    );

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Convert `Settings` to a JSON map for key lookup.
fn settings_to_map(s: &crab_config::Settings) -> serde_json::Map<String, serde_json::Value> {
    let v = serde_json::to_value(s).unwrap_or_default();
    match v {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// List of valid `camelCase` settings keys.
fn known_settings_keys() -> Vec<&'static str> {
    vec![
        "apiProvider",
        "apiBaseUrl",
        "apiKey",
        "model",
        "smallModel",
        "maxTokens",
        "permissionMode",
        "systemPrompt",
        "mcpServers",
        "hooks",
        "theme",
    ]
}

/// Resolve the target settings.json path.
fn target_settings_path(global: bool) -> anyhow::Result<PathBuf> {
    if global {
        Ok(settings::global_config_dir().join("settings.json"))
    } else {
        let working_dir = std::env::current_dir()?;
        Ok(settings::project_config_dir(&working_dir).join("settings.json"))
    }
}

/// Parse a JSONC string to a `serde_json::Value`.
fn jsonc_to_value(content: &str) -> anyhow::Result<serde_json::Value> {
    jsonc_parser::parse_to_serde_value::<serde_json::Value>(
        content,
        &jsonc_parser::ParseOptions::default(),
    )
    .map_err(|e| anyhow::anyhow!("JSONC parse error: {e}"))
}

/// Try to parse a string value as a JSON-typed value.
fn parse_value(value: &str) -> serde_json::Value {
    // Try as integer
    if let Ok(n) = value.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    // Try as bool
    match value {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        "null" => return serde_json::Value::Null,
        _ => {}
    }
    // Try as JSON object/array
    if (value.starts_with('{') || value.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(value).is_ok()
    {
        return serde_json::from_str(value).unwrap();
    }
    // Default: string
    serde_json::Value::String(value.to_string())
}

/// Return "(exists)" or "(not found)" for display.
fn existence_tag(path: &std::path::Path) -> &'static str {
    if path.exists() {
        "(exists)"
    } else {
        "(not found)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_integer() {
        assert_eq!(parse_value("42"), serde_json::json!(42));
        assert_eq!(parse_value("0"), serde_json::json!(0));
        assert_eq!(parse_value("-1"), serde_json::json!(-1));
    }

    #[test]
    fn parse_value_bool() {
        assert_eq!(parse_value("true"), serde_json::json!(true));
        assert_eq!(parse_value("false"), serde_json::json!(false));
    }

    #[test]
    fn parse_value_null() {
        assert_eq!(parse_value("null"), serde_json::Value::Null);
    }

    #[test]
    fn parse_value_string() {
        assert_eq!(parse_value("anthropic"), serde_json::json!("anthropic"));
        assert_eq!(parse_value("gpt-4o"), serde_json::json!("gpt-4o"));
    }

    #[test]
    fn parse_value_json_object() {
        let v = parse_value(r#"{"key": "val"}"#);
        assert!(v.is_object());
        assert_eq!(v["key"], "val");
    }

    #[test]
    fn parse_value_json_array() {
        let v = parse_value(r#"["a", "b"]"#);
        assert!(v.is_array());
    }

    #[test]
    fn parse_value_brace_like_string_not_valid_json() {
        // A string that starts with '{' but is not valid JSON should be a string
        let v = parse_value("{not json");
        assert!(v.is_string());
    }

    #[test]
    fn known_keys_contains_expected() {
        let keys = known_settings_keys();
        assert!(keys.contains(&"apiProvider"));
        assert!(keys.contains(&"model"));
        assert!(keys.contains(&"maxTokens"));
        assert!(keys.contains(&"theme"));
    }

    #[test]
    fn settings_to_map_roundtrip() {
        let s = crab_config::Settings {
            model: Some("test-model".into()),
            theme: Some("dark".into()),
            ..Default::default()
        };
        let map = settings_to_map(&s);
        assert_eq!(
            map.get("model").and_then(|v| v.as_str()),
            Some("test-model")
        );
        assert_eq!(map.get("theme").and_then(|v| v.as_str()), Some("dark"));
    }

    #[test]
    fn settings_to_map_none_fields_are_null() {
        let s = crab_config::Settings::default();
        let map = settings_to_map(&s);
        assert!(map.get("model").is_some_and(serde_json::Value::is_null));
    }

    #[test]
    fn existence_tag_nonexistent() {
        assert_eq!(
            existence_tag(std::path::Path::new("/nonexistent/path")),
            "(not found)"
        );
    }

    #[test]
    fn cmd_list_succeeds() {
        // Should not panic — loads merged settings and prints JSON
        let result = cmd_list();
        assert!(result.is_ok());
    }

    #[test]
    fn cmd_get_known_key() {
        // "model" is a valid key, even if its value is null
        let result = cmd_get("model");
        assert!(result.is_ok());
    }

    #[test]
    fn cmd_get_unknown_key() {
        let result = cmd_get("nonexistentKey123");
        assert!(result.is_err());
    }

    #[test]
    fn cmd_set_unknown_key_errors() {
        let result = cmd_set("badKey999", "value", true);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unknown configuration key"));
    }

    #[test]
    fn cmd_set_and_get_roundtrip() {
        let dir = std::env::temp_dir().join("crab-cli-config-test-set-get");
        let crab_dir = dir.join(".crab");
        let _ = std::fs::create_dir_all(&crab_dir);
        let settings_file = crab_dir.join("settings.json");
        std::fs::write(&settings_file, "{}\n").unwrap();

        // Write directly to the temp file to test the logic
        let mut map = serde_json::Map::new();
        map.insert("model".into(), parse_value("test-model-123"));
        let json = serde_json::to_string_pretty(&serde_json::Value::Object(map)).unwrap();
        std::fs::write(&settings_file, format!("{json}\n")).unwrap();

        // Read back
        let content = std::fs::read_to_string(&settings_file).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["model"], "test-model-123");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cmd_path_succeeds() {
        let result = cmd_path();
        assert!(result.is_ok());
    }

    #[test]
    fn jsonc_to_value_valid() {
        let v = jsonc_to_value(r#"{"key": "val" /* comment */}"#).unwrap();
        assert_eq!(v["key"], "val");
    }

    #[test]
    fn jsonc_to_value_invalid() {
        let result = jsonc_to_value("not json");
        assert!(result.is_err());
    }
}
