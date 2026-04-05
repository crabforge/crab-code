<div align="center">

# Crab Code

**Open-source alternative to Claude Code, built from scratch in Rust.**

*Inspired by Claude Code's agentic workflow -- open source, Rust-native, works with any LLM.*

[![Rust](https://img.shields.io/badge/Built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](#contributing)

</div>

---

> **Status: Active Development (Phase 1 nearing completion)** -- Core agent loop, tools, TUI, MCP, and multi-agent foundations are implemented. 945+ tests passing across 17 crates.

## What is Crab Code?

[Claude Code](https://docs.anthropic.com/en/docs/claude-code) pioneered the agentic coding CLI -- an AI that doesn't just suggest code, but thinks, plans, and executes autonomously in your terminal.

**Crab Code** brings this agentic coding experience to the open-source world, independently built from the ground up in Rust:

- **Fully open source** -- Apache 2.0, no feature-gating, no black box
- **Rust-native performance** -- instant startup, minimal memory, no Node.js overhead
- **Model agnostic** -- Claude, GPT, DeepSeek, Qwen, Ollama, or any OpenAI-compatible API
- **Secure** -- permission control with Default/TrustProject/Dangerously modes
- **MCP compatible** -- stdio and SSE transports, bridges MCP tools to native tool system

## Quick Start

```bash
git clone https://github.com/crabforge/crab-code.git
cd crab-code
cargo build --release

# Interactive TUI mode
./target/release/crab

# Single-shot mode
./target/release/crab "explain this codebase"

# With a specific provider
./target/release/crab --provider openai --model gpt-4o "fix the bug in main.rs"
```

## Features

### Implemented (Phase 1)

- [x] Agent loop -- model reasoning + tool execution cycle with SSE streaming
- [x] Built-in tools -- Read, Write, Edit, Bash, Glob, Grep (+ AgentTool, TaskTools)
- [x] Permission system -- Default / TrustProject / Dangerously modes with glob matching
- [x] `CRAB.md` support -- project instructions, multi-level (global + project)
- [x] LLM providers -- Anthropic Messages API + OpenAI-compatible (Ollama/DeepSeek/vLLM)
- [x] MCP client -- stdio and SSE transports, McpToolAdapter bridging
- [x] Conversation history -- session save/load/resume
- [x] Context window management -- auto-compaction at 80% threshold
- [x] Memory system -- persistent memory with frontmatter metadata
- [x] Interactive terminal UI (ratatui) -- Markdown rendering, syntax highlighting, Vim mode
- [x] Skill system -- skill discovery, /command triggers, hook execution
- [x] Sub-agent workers -- AgentTool spawns independent sub-agents
- [x] Task system -- TaskCreate/Get/List/Update with dependency graph

### Planned (Phase 2)

- [ ] OS-level sandboxing -- Landlock (Linux) / Seatbelt (macOS)
- [ ] OAuth2 + AWS Bedrock / GCP Vertex authentication
- [ ] Team/swarm multi-agent coordination
- [ ] WASM plugin runtime
- [ ] WebSocket MCP transport
- [ ] Daemon mode for background tasks
- [ ] Auto-update mechanism

## Architecture

4-layer, 17-crate Rust workspace:

```
Layer 4 (Entry)     cli          daemon        xtask
                      |              |
Layer 3 (Orch)     agent         session
                      |              |
Layer 2 (Service)  api   tools   mcp   tui   plugin   telemetry
                      |     |      |     |      |         |
Layer 1 (Found)    common   core   config   auth
```

Key design decisions:
- **Async runtime**: tokio (multi-threaded)
- **LLM dispatch**: `enum LlmBackend` -- zero dynamic dispatch, exhaustive match
- **Tool system**: `trait Tool` with JSON Schema discovery, `ToolRegistry` + `ToolExecutor`
- **TUI**: ratatui + crossterm, immediate-mode rendering
- **Error handling**: `thiserror` for libraries, `anyhow` for application

> Full architecture details: [`docs/architecture.md`](docs/architecture.md)

## Configuration

Crab Code uses its own independent configuration paths:

```bash
# Global config
~/.crab/settings.json        # API keys, provider settings, MCP servers
~/.crab/memory/              # Persistent memory files
~/.crab/sessions/            # Saved conversation sessions
~/.crab/skills/              # Global skill definitions

# Project config
your-project/CRAB.md         # Project instructions (like CLAUDE.md)
your-project/.crab/settings.json  # Project-level overrides
your-project/.crab/skills/   # Project-specific skills
```

```json
// ~/.crab/settings.json
{
  "apiProvider": "anthropic",
  "model": "claude-sonnet-4-20250514",
  "permissionMode": "default",
  "mcpServers": {
    "my-server": {
      "command": "npx",
      "args": ["-y", "@my/mcp-server"]
    }
  }
}
```

## CLI Usage

```bash
crab                          # Interactive TUI mode
crab "your prompt"            # Single-shot mode
crab --provider openai        # Use OpenAI-compatible provider
crab --model gpt-4o           # Override model
crab -t                       # Trust project permissions
crab --resume <session-id>    # Resume a saved session
crab session list             # List saved sessions
crab session show <id>        # Show session transcript
```

## Build & Development

```bash
cargo build --workspace                # Build all
cargo test --workspace                 # Run all tests (945+)
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all --check                # Check formatting
cargo run --bin crab                   # Run CLI
```

## Comparison

| | Crab Code | Claude Code | Codex CLI |
|--|-----------|-------------|-----------|
| Open Source | Apache 2.0 | Proprietary | Apache 2.0 |
| Language | Rust | TypeScript (Node.js) | Rust |
| Model Agnostic | Any provider | Anthropic + OpenAI-compat | OpenAI only |
| Self-hosted | Yes | No | Yes |
| MCP Support | stdio + SSE | 6 transports | 2 transports |
| TUI Framework | ratatui | Ink (React) | ratatui |

## Roadmap

```
M0  Project scaffold + CI       [##########]  Done
M1  Domain models (core+common) [##########]  Done -- 85+ types
M2  Streaming API (api+auth)    [##########]  Done -- Anthropic + OpenAI SSE
M3  Core tools (tools+fs+proc)  [##########]  Done -- 8 built-in tools
M4  Agent loop (session+agent)  [##########]  Done -- query loop + REPL [Dogfooding]
M5  Terminal UI (tui)           [##########]  Done -- ratatui interactive TUI
M6  Config + context mgmt       [##########]  Done -- permissions, CRAB.md, memory
M7a MCP integration             [##########]  Done -- MCP client + McpToolAdapter
M7b Multi-agent + skills        [########--]  In progress -- AgentTool, tasks, skills
```

## Contributing

We'd love your help! Crab Code is built independently from scratch.

```
Areas we need help with:
+-- Testing & benchmarks
+-- OS-level sandboxing (Landlock/Seatbelt)
+-- Additional LLM provider integrations
+-- MCP WebSocket transport
+-- Documentation & i18n
+-- Plugin system (WASM runtime)
```

## License

[Apache License 2.0](LICENSE)

---

<div align="center">

**Built with Rust by the [CrabForge](https://github.com/crabforge) community**

*Claude Code showed us the future of agentic coding. Crab Code makes it open for everyone.*

</div>
