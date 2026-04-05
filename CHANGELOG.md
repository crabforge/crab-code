# Changelog

All notable changes to Crab Code will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-05

### Added

**Core Architecture (Layer 1)**
- `crab-common`: Shared error types (`thiserror`), text utilities, path normalization (`dunce`), ULID generation
- `crab-core`: Domain models — `Message`, `Conversation`, `Tool` trait, `ToolRegistry`, `ContentBlock`, `TokenUsage`, `ModelId`, permission model (`PermissionMode`, `PermissionPolicy`), event system
- `crab-config`: Multi-layer configuration — `Settings` struct with JSONC parsing, `CRAB.md` project instructions, hook definitions, feature flags, keybindings; multi-provider TOML config (`~/.config/crab-code/config.toml`); environment variable overrides (`CRAB_API_PROVIDER`, `CRAB_API_KEY`, `CRAB_MODEL`, `CRAB_API_BASE_URL`); merge priority chain (config.toml < global settings < project settings < env vars)
- `crab-auth`: API key resolution (explicit > env var > keychain), system keychain integration (`keyring`), `AuthProvider` trait, provider-specific env var routing (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`)

**API Clients (Layer 2)**
- `crab-api`: Dual LLM backend — Anthropic Messages API + OpenAI-compatible Chat Completions; SSE streaming; automatic retry with exponential backoff; rate limiting; prompt caching support; `LlmBackend` enum dispatch (no dynamic dispatch)

**Tool System (Layer 2)**
- `crab-tools`: Built-in tool implementations — `BashTool`, `ReadTool`, `WriteTool`, `EditTool`, `GlobTool`, `GrepTool`, `WebSearchTool`, `WebFetchTool`, `AgentTool`, `TaskCreateTool`, `TaskListTool`, `TaskGetTool`, `TaskUpdateTool`; `ToolExecutor` with permission checking; `ToolRegistry` with JSON Schema discovery
- `crab-fs`: File system operations — glob matching (`globset`), content search (`regex`), `.gitignore`-aware walking (`ignore`), unified diff generation (`similar`)
- `crab-process`: Child process management — async spawn with output capture, process tree tracking, signal handling
- `crab-mcp`: Model Context Protocol — JSON-RPC client, stdio/SSE/WebSocket transports, `McpToolAdapter` bridging MCP tools to native `Tool` trait, server lifecycle management

**Plugin System (Layer 2)**
- `crab-plugin`: Skill system — YAML front-matter skill files, `SkillRegistry` with command/prefix triggers, skill discovery from global + project directories; hook execution framework

**Session & Agent (Layer 3)**
- `crab-session`: Session state management, conversation history, context window tracking, token usage accumulation, context compaction (80% threshold), session persistence (save/load/delete)
- `crab-agent`: Core agent loop — user input > system prompt + history > LLM API (SSE streaming) > tool call parsing > permission check > tool execution > result serialization > next turn; system prompt builder (CRAB.md + tool descriptions + env info); memory system integration; multi-agent coordination with `AgentCoordinator` and worker lifecycle

**Terminal Interface (Layer 2/4)**
- `crab-tui`: Terminal UI with `ratatui` — Markdown rendering, syntax highlighting (`syntect`), ANSI support, permission confirmation dialogs, tool output display, Vim mode basics
- `crab-cli`: CLI entry point (`clap`) — single-shot mode, interactive REPL, `/command` skill resolution, session management subcommands (`list`, `show`, `resume`, `delete`), permission mode flags (`--trust-project`, `--dangerously-skip-permissions`)

**Observability (Layer 2)**
- `crab-telemetry`: Local-only tracing initialization (`tracing-subscriber`), structured logging with env filter

**Configuration Paths**
- `~/.crab/settings.json` — global settings
- `.crab/settings.json` — project-level settings
- `~/.config/crab-code/config.toml` — multi-provider configuration
- `CRAB.md` — project instructions (equivalent to CLAUDE.md)
- `~/.crab/memory/` — persistent memory store
- `~/.crab/sessions/` — session history
- `~/.crab/skills/` — global skills

[0.1.0]: https://github.com/CrabForge/crab-code/releases/tag/v0.1.0
