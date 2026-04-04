mod commands;
mod setup;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use crab_agent::{AgentSession, SessionConfig, build_system_prompt};
use crab_core::event::Event;
use crab_core::model::ModelId;
use crab_core::permission::PermissionPolicy;
use crab_tools::builtin::create_default_registry;
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = crab_config::Settings {
        api_provider: Some(cli.provider.clone()),
        model: cli.model.clone(),
        ..Default::default()
    };

    let model_id = cli.model.unwrap_or_else(|| {
        if cli.provider == "openai" {
            "gpt-4o".to_string()
        } else {
            "claude-sonnet-4-20250514".to_string()
        }
    });

    let backend = Arc::new(crab_api::create_backend(&settings));
    let registry = create_default_registry();

    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let system_prompt = build_system_prompt(
        &working_dir,
        &registry,
        settings.system_prompt.as_deref(),
    );

    let session_config = SessionConfig {
        session_id: crab_common::id::new_ulid(),
        system_prompt,
        model: ModelId::from(model_id.as_str()),
        max_tokens: cli.max_tokens,
        temperature: None,
        context_window: 200_000,
        working_dir,
        permission_policy: PermissionPolicy::default(),
    };

    eprintln!(
        "crab-code v{} (provider={}, model={model_id})",
        env!("CARGO_PKG_VERSION"),
        cli.provider,
    );

    if let Some(prompt) = cli.prompt {
        // Single-shot mode: create an AgentSession and run once
        let mut session = AgentSession::new(session_config, backend, registry);
        run_single_shot(&mut session, &prompt).await
    } else {
        // Interactive mode: TUI if available, else line-based REPL
        #[cfg(feature = "tui")]
        {
            let tui_config = crab_tui::TuiConfig {
                session_config,
                backend,
            };
            crab_tui::run(tui_config).await
        }
        #[cfg(not(feature = "tui"))]
        {
            let mut session = AgentSession::new(session_config, backend, registry);
            eprintln!("Type /exit or Ctrl+D to quit.\n");
            run_repl(&mut session).await
        }
    }
}

/// Run a single prompt, print the result, and exit.
async fn run_single_shot(session: &mut AgentSession, prompt: &str) -> anyhow::Result<()> {
    let event_rx = take_event_rx(session);
    let printer = tokio::spawn(print_events(event_rx));

    let result = session.handle_user_input(prompt).await;
    // Drop the event_tx side so printer finishes
    drop(session.event_tx.clone());
    let _ = printer.await;

    result.map_err(Into::into)
}

/// Interactive REPL: read lines, send to agent, print streaming output.
#[cfg(not(feature = "tui"))]
async fn run_repl(session: &mut AgentSession) -> anyhow::Result<()> {
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

        let event_rx = take_event_rx(session);
        let printer = tokio::spawn(print_events(event_rx));

        match session.handle_user_input(input).await {
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
async fn print_events(mut rx: mpsc::Receiver<Event>) {
    let mut stdout = std::io::stdout();
    while let Some(event) = rx.recv().await {
        match event {
            Event::ContentDelta { delta, .. } => {
                print!("{delta}");
                let _ = stdout.flush();
            }
            Event::ToolUseStart { name, .. } => {
                eprintln!("\n[tool] {name}");
            }
            Event::ToolResult { id: _, output } => {
                if output.is_error {
                    eprintln!("[tool error] {}", output.text());
                } else {
                    let text = output.text();
                    if text.len() > 500 {
                        eprintln!("[tool result] {}...", &text[..500]);
                    } else {
                        eprintln!("[tool result] {text}");
                    }
                }
            }
            Event::Error { message } => {
                eprintln!("[error] {message}");
            }
            Event::TokenWarning {
                usage_pct,
                used,
                limit,
            } => {
                eprintln!(
                    "[warn] Token usage {:.0}% ({used}/{limit})",
                    usage_pct * 100.0,
                );
            }
            Event::CompactStart { strategy, .. } => {
                eprintln!("[compact] Starting compaction: {strategy}");
            }
            Event::CompactEnd {
                after_tokens,
                removed_messages,
            } => {
                eprintln!(
                    "[compact] Compacted: removed {removed_messages} messages, now {after_tokens} tokens"
                );
            }
            Event::TurnStart { .. }
            | Event::MessageStart { .. }
            | Event::ContentBlockStop { .. }
            | Event::MessageEnd { .. }
            | Event::ToolUseInput { .. }
            | Event::PermissionRequest { .. }
            | Event::PermissionResponse { .. } => {}
        }
    }
}
