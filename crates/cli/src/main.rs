mod commands;
mod completions;
mod json_output;
mod output;
mod setup;

use std::io::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use crab_agent::{AgentSession, SessionConfig, build_system_prompt};
use crab_core::event::Event;
use crab_core::model::ModelId;
use crab_core::permission::{PermissionMode, PermissionPolicy};
use crab_tools::builtin::create_default_registry;
use crab_tools::executor::PermissionHandler;
use tokio::sync::mpsc;

/// Crab Code -- Rust-native Agentic Coding CLI
#[derive(Parser)]
#[command(name = "crab", version, about)]
struct Cli {
    /// User prompt (if provided, runs single-shot mode then exits)
    prompt: Option<String>,

    /// LLM provider: "anthropic" (default) or "openai"
    #[arg(long, default_value = "anthropic")]
    provider: String,

    /// Model ID override (e.g. "claude-sonnet-4-20250514", "gpt-4o")
    #[arg(long, short)]
    model: Option<String>,

    /// Maximum output tokens
    #[arg(long, default_value = "4096")]
    max_tokens: u32,

    /// Trust in-project file operations (skip confirmation for project writes)
    #[arg(long, short = 't')]
    trust_project: bool,

    /// Skip ALL permission checks (dangerous!)
    #[arg(long)]
    dangerously_skip_permissions: bool,

    /// Output results as newline-delimited JSON (machine-readable)
    #[arg(long)]
    json: bool,

    /// Resume a previous session by ID
    #[arg(long)]
    resume: Option<String>,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

/// Subcommands for `crab`.
#[derive(Subcommand)]
enum CliCommand {
    /// Manage saved sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Run as an MCP server (expose tools to external MCP clients)
    Serve(commands::serve::ServeArgs),
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: commands::config::ConfigAction,
    },
    /// Initialize a new project from a template
    Init(commands::init::InitArgs),
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, powershell)
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Session management actions.
#[derive(Subcommand)]
enum SessionAction {
    /// List all saved sessions
    List,
    /// Show the transcript of a saved session
    Show {
        /// Session ID to display
        id: String,
    },
    /// Resume a saved session (alias for `crab --resume <id>`)
    Resume {
        /// Session ID to resume
        id: String,
    },
    /// Delete a saved session
    Delete {
        /// Session ID to delete
        id: String,
    },
    /// Search history sessions for a keyword
    Search {
        /// Keyword to search for
        keyword: String,
    },
    /// Export a session to JSON or Markdown
    Export {
        /// Session ID to export
        id: String,
        /// Output format: "json" or "markdown" (default: markdown)
        #[arg(long, default_value = "markdown")]
        format: String,
    },
    /// Show statistics for a session
    Stats {
        /// Session ID
        id: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle subcommands
    if let Some(command) = &cli.command {
        return match command {
            CliCommand::Completions { shell } => {
                completions::generate_completions(*shell);
                Ok(())
            }
            CliCommand::Config { action } => commands::config::run(action),
            CliCommand::Init(args) => commands::init::run(args),
            CliCommand::Serve(args) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(commands::serve::run(args))
            }
            CliCommand::Session { action } => match action {
                SessionAction::List => commands::session::list_sessions(),
                SessionAction::Show { id } => commands::session::show_session(id),
                SessionAction::Resume { id } => {
                    // Validate, then fall through to run the session
                    let _ = commands::session::validate_resume_id(id)?;
                    let rt = tokio::runtime::Runtime::new()?;
                    rt.block_on(run_with_resume(&cli, Some(id.clone())))
                }
                SessionAction::Delete { id } => commands::session::delete_session(id),
                SessionAction::Search { keyword } => commands::session::search_sessions(keyword),
                SessionAction::Export { id, format } => {
                    commands::session::export_session(id, format)
                }
                SessionAction::Stats { id } => commands::session::show_stats(id),
            },
        };
    }

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run(&cli, cli.resume.clone()))
}

/// Convenience wrapper for `Session resume` subcommand.
async fn run_with_resume(cli: &Cli, resume_id: Option<String>) -> anyhow::Result<()> {
    run(cli, resume_id).await
}

#[allow(clippy::too_many_lines)]
async fn run(cli: &Cli, resume_session_id: Option<String>) -> anyhow::Result<()> {
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Load merged settings (global ~/.crab/settings.json + project .crab/settings.json)
    let settings = crab_config::settings::load_merged_settings(Some(&working_dir))?;

    // CLI args override settings; non-default CLI provider overrides settings
    let provider = if cli.provider == "anthropic" {
        settings
            .api_provider
            .clone()
            .unwrap_or_else(|| cli.provider.clone())
    } else {
        cli.provider.clone()
    };
    let model_id = cli
        .model
        .clone()
        .or_else(|| settings.model.clone())
        .unwrap_or_else(|| {
            if provider == "openai" {
                "gpt-4o".to_string()
            } else {
                "claude-sonnet-4-20250514".to_string()
            }
        });

    // Build effective settings for backend creation
    let effective_settings = crab_config::Settings {
        api_provider: Some(provider.clone()),
        api_base_url: settings.api_base_url.clone(),
        api_key: settings.api_key.clone(),
        model: Some(model_id.clone()),
        ..settings.clone()
    };

    let backend = Arc::new(crab_api::create_backend(&effective_settings));
    let registry = create_default_registry();

    // Discover skills from global + project directories
    let skill_dirs = build_skill_dirs(&working_dir);
    let skill_registry =
        crab_plugin::skill::SkillRegistry::discover(&skill_dirs).unwrap_or_default();
    if !skill_registry.is_empty() {
        eprintln!("Loaded {} skill(s).", skill_registry.len());
    }

    // Build system prompt (includes CRAB.md + tool descriptions + env info)
    let system_prompt = build_system_prompt(
        &working_dir,
        &registry,
        effective_settings.system_prompt.as_deref(),
    );

    // Resolve permission mode: CLI flags > settings file > default
    let permission_mode = if cli.dangerously_skip_permissions {
        PermissionMode::Dangerously
    } else if cli.trust_project {
        PermissionMode::TrustProject
    } else {
        match settings.permission_mode.as_deref() {
            Some("trust-project" | "trust_project") => PermissionMode::TrustProject,
            Some("dangerously") => PermissionMode::Dangerously,
            _ => PermissionMode::Default,
        }
    };

    let global_dir = crab_config::settings::global_config_dir();
    let session_config = SessionConfig {
        session_id: crab_common::id::new_ulid(),
        system_prompt,
        model: ModelId::from(model_id.as_str()),
        max_tokens: cli.max_tokens,
        temperature: None,
        context_window: 200_000,
        working_dir,
        permission_policy: PermissionPolicy {
            mode: permission_mode,
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
        },
        memory_dir: Some(global_dir.join("memory")),
        sessions_dir: Some(global_dir.join("sessions")),
        resume_session_id,
    };

    output::banner(
        env!("CARGO_PKG_VERSION"),
        &provider,
        &model_id,
        &permission_mode,
    );

    if let Some(ref prompt) = cli.prompt {
        // Single-shot mode: check if it's a /command
        let effective_prompt = resolve_slash_command(prompt, &skill_registry);
        let mut session = AgentSession::new(session_config, backend, registry);
        session
            .executor
            .set_permission_handler(Arc::new(CliPermissionHandler));
        run_single_shot(&mut session, &effective_prompt, cli.json).await
    } else {
        // Interactive mode: TUI if available, else line-based REPL
        #[cfg(feature = "tui")]
        {
            let tui_config = crab_tui::TuiConfig {
                session_config,
                backend,
                skill_dirs,
            };
            crab_tui::run(tui_config).await
        }
        #[cfg(not(feature = "tui"))]
        {
            let mut session = AgentSession::new(session_config, backend, registry);
            session
                .executor
                .set_permission_handler(Arc::new(CliPermissionHandler));
            eprintln!("Type /exit or Ctrl+D to quit.\n");
            run_repl(&mut session, &skill_registry).await
        }
    }
}

/// Build the list of skill directories to scan.
fn build_skill_dirs(working_dir: &std::path::Path) -> Vec<PathBuf> {
    // Global skills: ~/.crab/skills/
    // Project skills: <project>/.crab/skills/
    vec![
        crab_config::settings::global_config_dir().join("skills"),
        working_dir.join(".crab").join("skills"),
    ]
}

/// If input starts with `/`, try to match a skill command and return its content
/// as the prompt. Otherwise return the original input.
fn resolve_slash_command(
    input: &str,
    skill_registry: &crab_plugin::skill::SkillRegistry,
) -> String {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return input.to_string();
    }

    // Extract the command name (first word after /)
    let command = trimmed
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("");

    // Check built-in commands first
    if matches!(command, "exit" | "quit" | "help") {
        return input.to_string();
    }

    // Look up in skill registry
    if let Some(skill) = skill_registry.find_command(command) {
        // The rest of the input after the /command becomes arguments
        let args = trimmed
            .trim_start_matches('/')
            .trim_start_matches(command)
            .trim();

        let mut prompt = skill.content.clone();
        if !args.is_empty() {
            prompt.push_str("\n\nUser arguments: ");
            prompt.push_str(args);
        }

        eprintln!("[skill] Activated: {} — {}", skill.name, skill.description);
        return prompt;
    }

    // No matching skill — pass through as-is
    input.to_string()
}

/// CLI-based permission handler: prints prompt to stderr, reads y/n from stdin.
struct CliPermissionHandler;

impl PermissionHandler for CliPermissionHandler {
    fn ask_permission(
        &self,
        tool_name: &str,
        prompt: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        let tool_name = tool_name.to_string();
        let prompt = prompt.to_string();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                use std::io::BufRead;
                eprint!("[permission] {prompt} ({tool_name}) [y/N] ");
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                if std::io::stdin().lock().read_line(&mut line).is_ok() {
                    let answer = line.trim().to_lowercase();
                    answer == "y" || answer == "yes"
                } else {
                    false
                }
            })
            .await
            .unwrap_or(false)
        })
    }
}

/// Run a single prompt, print the result, and exit.
async fn run_single_shot(
    session: &mut AgentSession,
    prompt: &str,
    json_mode: bool,
) -> anyhow::Result<()> {
    let event_rx = take_event_rx(session);
    let printer = tokio::spawn(print_events(event_rx, json_mode));

    let result = session.handle_user_input(prompt).await;
    // Drop the event_tx side so printer finishes
    drop(session.event_tx.clone());
    let _ = printer.await;

    result.map_err(Into::into)
}

/// Interactive REPL: read lines, send to agent, print streaming output.
#[cfg(not(feature = "tui"))]
async fn run_repl(
    session: &mut AgentSession,
    skill_registry: &crab_plugin::skill::SkillRegistry,
) -> anyhow::Result<()> {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        // Print prompt
        print!("crab> ");
        stdout.flush()?;

        // Read a line
        let mut line = String::new();
        let bytes_read = stdin.lock().read_line(&mut line)?;

        // Ctrl+D (EOF)
        if bytes_read == 0 {
            eprintln!("\nGoodbye!");
            break;
        }

        let input = line.trim();

        if input.is_empty() {
            continue;
        }

        if input == "/exit" || input == "/quit" {
            eprintln!("Goodbye!");
            break;
        }

        // Resolve /command to skill content
        let effective_input = resolve_slash_command(input, skill_registry);

        let event_rx = take_event_rx(session);
        let printer = tokio::spawn(print_events(event_rx, false));

        match session.handle_user_input(&effective_input).await {
            Ok(()) => {}
            Err(e) => {
                eprintln!("\n[error] {e}");
            }
        }

        let _ = printer.await;
        println!();
    }

    Ok(())
}

/// Swap the session's `event_rx` with a fresh one, returning the old receiver.
fn take_event_rx(session: &mut AgentSession) -> mpsc::Receiver<Event> {
    let (new_tx, new_rx) = mpsc::channel(256);
    let old_rx = std::mem::replace(&mut session.event_rx, new_rx);
    session.event_tx = new_tx;
    old_rx
}

/// Drain events from the receiver and print them to stdout/stderr.
///
/// When `json_mode` is true, emits newline-delimited JSON to stdout.
/// Otherwise uses colored human-readable output.
async fn print_events(mut rx: mpsc::Receiver<Event>, json_mode: bool) {
    let mut stdout = std::io::stdout();
    let mut spinner: Option<output::Spinner> = None;

    while let Some(event) = rx.recv().await {
        if json_mode {
            if let Some(value) = json_output::event_to_json(&event) {
                json_output::print_json_line(&value);
            }
            continue;
        }

        match event {
            Event::ContentDelta { delta, .. } => {
                // Stop spinner when content starts streaming
                if let Some(mut s) = spinner.take() {
                    s.stop();
                }
                print!("{delta}");
                let _ = stdout.flush();
            }
            Event::ToolUseStart { name, .. } => {
                // Stop any previous spinner before starting a new one
                if let Some(mut s) = spinner.take() {
                    s.stop();
                }
                output::tool_use(&name);
                spinner = Some(output::Spinner::start(&format!("running {name}...")));
            }
            Event::ToolResult { id: _, output: o } => {
                if let Some(mut s) = spinner.take() {
                    s.stop();
                }
                output::tool_result(&o.text(), o.is_error);
            }
            Event::Error { message } => {
                output::error(&message);
            }
            Event::TokenWarning {
                usage_pct,
                used,
                limit,
            } => {
                output::token_warning(usage_pct, used, limit);
            }
            Event::CompactStart { strategy, .. } => {
                output::compact_start(&strategy);
            }
            Event::CompactEnd {
                after_tokens,
                removed_messages,
            } => {
                output::compact_end(removed_messages, after_tokens);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_plugin::skill::{Skill, SkillRegistry, SkillTrigger};

    #[test]
    fn build_skill_dirs_includes_global_and_project() {
        let dirs = build_skill_dirs(std::path::Path::new("/tmp/project"));
        // Should contain at least the project skills dir
        assert!(dirs.iter().any(|d| d.ends_with(".crab/skills")));
    }

    #[test]
    fn resolve_slash_command_passthrough_non_slash() {
        let reg = SkillRegistry::new();
        assert_eq!(resolve_slash_command("hello world", &reg), "hello world");
    }

    #[test]
    fn resolve_slash_command_builtin_passthrough() {
        let reg = SkillRegistry::new();
        assert_eq!(resolve_slash_command("/exit", &reg), "/exit");
        assert_eq!(resolve_slash_command("/quit", &reg), "/quit");
        assert_eq!(resolve_slash_command("/help", &reg), "/help");
    }

    #[test]
    fn resolve_slash_command_no_match_passthrough() {
        let reg = SkillRegistry::new();
        assert_eq!(
            resolve_slash_command("/unknown-skill", &reg),
            "/unknown-skill"
        );
    }

    #[test]
    fn resolve_slash_command_matches_skill() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill {
            name: "commit".into(),
            description: "Create a commit".into(),
            trigger: SkillTrigger::Command {
                name: "commit".into(),
            },
            content: "You are a commit helper.".into(),
            source_path: None,
        });

        let result = resolve_slash_command("/commit", &reg);
        assert_eq!(result, "You are a commit helper.");
    }

    #[test]
    fn resolve_slash_command_with_args() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill {
            name: "review".into(),
            description: "Review code".into(),
            trigger: SkillTrigger::Command {
                name: "review".into(),
            },
            content: "Review the code.".into(),
            source_path: None,
        });

        let result = resolve_slash_command("/review src/main.rs", &reg);
        assert!(result.contains("Review the code."));
        assert!(result.contains("src/main.rs"));
    }

    #[test]
    fn cli_parses_json_flag() {
        let cli = Cli::try_parse_from(["crab", "--json", "hello"]).unwrap();
        assert!(cli.json);
        assert_eq!(cli.prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn cli_json_defaults_to_false() {
        let cli = Cli::try_parse_from(["crab", "hello"]).unwrap();
        assert!(!cli.json);
    }

    #[test]
    fn cli_parses_completions_subcommand() {
        let cli = Cli::try_parse_from(["crab", "completions", "bash"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(CliCommand::Completions { shell: Shell::Bash })
        ));
    }

    #[test]
    fn cli_completions_all_shells() {
        for shell in ["bash", "zsh", "fish", "powershell"] {
            let cli = Cli::try_parse_from(["crab", "completions", shell]).unwrap();
            assert!(matches!(cli.command, Some(CliCommand::Completions { .. })));
        }
    }
}
