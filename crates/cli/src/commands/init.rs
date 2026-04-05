// Project scaffolding subcommand: `crab init`

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Args;

use crab_config::settings;
use crab_fs::template;

/// Arguments for `crab init`.
#[derive(Args)]
pub struct InitArgs {
    /// Template name to use (omit to list available templates)
    pub template: Option<String>,

    /// Project name (also used as the target directory name)
    #[arg(long, short)]
    pub name: Option<String>,

    /// Target directory (defaults to ./<name> or current directory)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Template variables in key=value format
    #[arg(long = "var", short = 'v')]
    pub vars: Vec<String>,

    /// List available templates and exit
    #[arg(long, short)]
    pub list: bool,
}

/// Execute the `crab init` command.
pub fn run(args: &InitArgs) -> anyhow::Result<()> {
    let templates_dir = templates_dir();

    // `crab init --list` or `crab init` (no template specified)
    if args.list || args.template.is_none() {
        return cmd_list(&templates_dir);
    }

    let template_name = args.template.as_deref().unwrap();
    cmd_scaffold(&templates_dir, template_name, args)
}

/// `crab init --list` — list all available templates.
fn cmd_list(templates_dir: &std::path::Path) -> anyhow::Result<()> {
    let templates = template::discover_templates(templates_dir)?;

    if templates.is_empty() {
        eprintln!("No templates found in {}", templates_dir.display());
        eprintln!("Create templates in ~/.crab/templates/<name>/template.toml");
        return Ok(());
    }

    println!("Available templates:\n");
    for t in &templates {
        let desc = if t.manifest.description.is_empty() {
            "(no description)"
        } else {
            &t.manifest.description
        };
        println!("  {:<20} {desc}", t.manifest.name);
    }

    println!("\nUse `crab init <template> --name <project>` to create a project.");
    Ok(())
}

/// `crab init <template>` — scaffold a project from a template.
fn cmd_scaffold(
    templates_dir: &std::path::Path,
    template_name: &str,
    args: &InitArgs,
) -> anyhow::Result<()> {
    let tmpl = template::find_template(templates_dir, template_name)?;

    // Parse --var key=value pairs
    let mut vars = parse_vars(&args.vars)?;

    // If --name is given, add it as both "project_name" and "name" variables
    if let Some(ref name) = args.name {
        vars.entry("project_name".to_string())
            .or_insert_with(|| name.clone());
        vars.entry("name".to_string())
            .or_insert_with(|| name.clone());
    }

    // Determine target directory
    let target_dir = resolve_target_dir(args)?;

    eprintln!(
        "Scaffolding '{}' into {}...",
        tmpl.manifest.name,
        target_dir.display()
    );

    let result = template::scaffold(&tmpl, &target_dir, &vars)?;

    eprintln!("Created {} file(s):", result.files_created.len());
    for f in &result.files_created {
        eprintln!("  {}", f.display());
    }

    if !result.post_create.is_empty() {
        eprintln!("\nPost-create commands:");
        for cmd in &result.post_create {
            eprintln!("  $ {cmd}");
        }
    }

    eprintln!("\nDone!");
    Ok(())
}

/// Parse `key=value` strings into a `HashMap`.
fn parse_vars(var_args: &[String]) -> anyhow::Result<HashMap<String, String>> {
    let mut vars = HashMap::new();
    for arg in var_args {
        let (key, value) = arg.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid variable format: '{arg}' (expected key=value)")
        })?;
        vars.insert(key.to_string(), value.to_string());
    }
    Ok(vars)
}

/// Resolve the target directory from CLI args.
fn resolve_target_dir(args: &InitArgs) -> anyhow::Result<PathBuf> {
    if let Some(ref output) = args.output {
        return Ok(output.clone());
    }

    if let Some(ref name) = args.name {
        let cwd = std::env::current_dir()?;
        return Ok(cwd.join(name));
    }

    // Default to current directory
    std::env::current_dir().map_err(Into::into)
}

/// Path to the templates directory: `~/.crab/templates/`
fn templates_dir() -> PathBuf {
    settings::global_config_dir().join("templates")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_vars_valid() {
        let args = vec!["name=my-app".to_string(), "license=MIT".to_string()];
        let vars = parse_vars(&args).unwrap();
        assert_eq!(vars.get("name").unwrap(), "my-app");
        assert_eq!(vars.get("license").unwrap(), "MIT");
    }

    #[test]
    fn parse_vars_empty() {
        let vars = parse_vars(&[]).unwrap();
        assert!(vars.is_empty());
    }

    #[test]
    fn parse_vars_invalid_format() {
        let args = vec!["no-equals-sign".to_string()];
        let result = parse_vars(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("key=value"));
    }

    #[test]
    fn parse_vars_value_with_equals() {
        let args = vec!["key=val=ue".to_string()];
        let vars = parse_vars(&args).unwrap();
        assert_eq!(vars.get("key").unwrap(), "val=ue");
    }

    #[test]
    fn resolve_target_dir_with_output() {
        let args = InitArgs {
            template: None,
            name: None,
            output: Some(PathBuf::from("/tmp/my-project")),
            vars: vec![],
            list: false,
        };
        let dir = resolve_target_dir(&args).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/my-project"));
    }

    #[test]
    fn resolve_target_dir_with_name() {
        let args = InitArgs {
            template: None,
            name: Some("my-app".to_string()),
            output: None,
            vars: vec![],
            list: false,
        };
        let dir = resolve_target_dir(&args).unwrap();
        assert!(dir.ends_with("my-app"));
    }

    #[test]
    fn resolve_target_dir_default_cwd() {
        let args = InitArgs {
            template: None,
            name: None,
            output: None,
            vars: vec![],
            list: false,
        };
        let dir = resolve_target_dir(&args).unwrap();
        assert_eq!(dir, std::env::current_dir().unwrap());
    }

    #[test]
    fn templates_dir_is_under_crab() {
        let dir = templates_dir();
        assert!(dir.to_string_lossy().contains(".crab"));
        assert!(dir.ends_with("templates"));
    }

    #[test]
    fn cmd_list_empty_templates_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Should succeed (prints "no templates found" to stderr)
        let result = cmd_list(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn cmd_list_with_templates() {
        let dir = tempfile::tempdir().unwrap();
        let t_dir = dir.path().join("my-tmpl");
        fs::create_dir(&t_dir).unwrap();
        fs::write(
            t_dir.join("template.toml"),
            "name = \"my-tmpl\"\ndescription = \"A test template\"\n",
        )
        .unwrap();

        let result = cmd_list(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn cmd_scaffold_success() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        // Create a template
        let t = tmpl_dir.path().join("rust-app");
        fs::create_dir(&t).unwrap();
        fs::write(
            t.join("template.toml"),
            r#"
name = "rust-app"
description = "Rust app template"

[[variables]]
name = "project_name"
required = true
"#,
        )
        .unwrap();
        fs::write(t.join("README.md"), "# {{project_name}}\n").unwrap();

        let args = InitArgs {
            template: Some("rust-app".to_string()),
            name: Some("hello-world".to_string()),
            output: Some(target_dir.path().to_path_buf()),
            vars: vec![],
            list: false,
        };

        let result = cmd_scaffold(tmpl_dir.path(), "rust-app", &args);
        assert!(result.is_ok());

        let readme = fs::read_to_string(target_dir.path().join("README.md")).unwrap();
        assert_eq!(readme, "# hello-world\n");
    }

    #[test]
    fn cmd_scaffold_with_extra_vars() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        let t = tmpl_dir.path().join("tmpl");
        fs::create_dir(&t).unwrap();
        fs::write(
            t.join("template.toml"),
            "name = \"tmpl\"\n\n[[variables]]\nname = \"author\"\nrequired = true\n",
        )
        .unwrap();
        fs::write(t.join("file.txt"), "by {{author}}\n").unwrap();

        let args = InitArgs {
            template: Some("tmpl".to_string()),
            name: None,
            output: Some(target_dir.path().to_path_buf()),
            vars: vec!["author=Alice".to_string()],
            list: false,
        };

        let result = cmd_scaffold(tmpl_dir.path(), "tmpl", &args);
        assert!(result.is_ok());

        let content = fs::read_to_string(target_dir.path().join("file.txt")).unwrap();
        assert_eq!(content, "by Alice\n");
    }

    #[test]
    fn cmd_scaffold_template_not_found() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let args = InitArgs {
            template: Some("nonexistent".to_string()),
            name: None,
            output: Some(tmpl_dir.path().to_path_buf()),
            vars: vec![],
            list: false,
        };

        let result = cmd_scaffold(tmpl_dir.path(), "nonexistent", &args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn cmd_scaffold_missing_required_var() {
        let tmpl_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        let t = tmpl_dir.path().join("req");
        fs::create_dir(&t).unwrap();
        fs::write(
            t.join("template.toml"),
            "name = \"req\"\n\n[[variables]]\nname = \"x\"\nrequired = true\n",
        )
        .unwrap();

        let args = InitArgs {
            template: Some("req".to_string()),
            name: None,
            output: Some(target_dir.path().to_path_buf()),
            vars: vec![],
            list: false,
        };

        let result = cmd_scaffold(tmpl_dir.path(), "req", &args);
        assert!(result.is_err());
    }
}
