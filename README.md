# pencode

The open source AI coding agent — rewritten in Rust.

pencode is a fork of [opencode](https://github.com/anomalyco/opencode) being
rebuilt as a native Rust application. This branch (`rust-rewrite`) replaces the
original TypeScript monorepo with a Cargo workspace that mirrors its
architecture.

## Status

This is an early-stage rewrite. The workspace compiles, tests pass, and the
core surfaces exist; full feature parity with upstream opencode is work in
progress.

| Crate | Replaces | Purpose |
|---|---|---|
| `crates/pencode-provider` | `packages/opencode/src/provider` | LLM clients: Anthropic Messages + OpenAI-compatible, streaming |
| `crates/pencode-protocol` | `packages/protocol` | Shared wire types: sessions, messages, parts, events |
| `crates/pencode-core` | `packages/opencode/src` | Config loading, durable session store, tool registry |
| `crates/pencode-server` | `packages/server` | HTTP API (axum): sessions, config, health |
| `crates/pencode-client` | `packages/sdk/js` | Typed HTTP client for the server API |
| `crates/pencode-tui` | `packages/tui` | Terminal UI (ratatui) with transcript + prompt |
| `crates/pencode` | CLI entrypoint | Subcommands: `run`, `serve`, `tui`, `models`, `auth` |

## Build

Requires Rust 1.85+ (edition 2021).

```
cargo build --release
cargo test --workspace
```

## Usage

```
pencode serve --port 4096   # start the HTTP API server
pencode tui                 # interactive terminal UI
pencode run "fix the bug"   # one-shot prompt
pencode models              # show configured model/providers
pencode auth                # provider auth status
```

## Configuration

Global: `~/.config/pencode/config.json` (respects `XDG_CONFIG_HOME`)
Project: `.pencode/config.json` in your repo (overrides global)

```json
{
  "theme": "dark",
  "model": "anthropic/claude-sonnet-4-5",
  "autoupdate": true,
  "provider": {
    "anthropic": { "apiKey": "sk-ant-..." }
  }
}
```

API keys can also come from the environment — `ANTHROPIC_API_KEY`,
`OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GROQ_API_KEY`. Any
OpenAI-compatible endpoint works by setting `provider.<name>.baseUrl`.

Supported model spec format is `provider/model`:

| Provider | Example | Protocol |
|---|---|---|
| `anthropic/...` | `anthropic/claude-sonnet-4-5` | Anthropic Messages |
| `openai/...` | `openai/gpt-4.1` | Chat Completions |
| `openrouter/...` | `openrouter/meta-llama/llama-3` | Chat Completions |
| `groq/...` | `groq/llama-3-70b` | Chat Completions |

Sessions are stored durably under `.pencode/storage/session/<id>.json`.
The HTTP server (`POST /session/:id/message`) and the TUI both generate
assistant replies through the configured provider, streaming into the UI.

## Roadmap

- [x] LLM provider integration (Anthropic, OpenAI-compatible) with streaming
- [ ] Tool execution inside model turns (read/write/bash wired to the agent loop)
- [ ] Full TUI feature parity (themes switching, mouse scroll)
- [ ] Desktop app (Tauri)

## License

MIT — see [LICENSE](LICENSE). pencode is a fork of opencode and is not built
by or affiliated with the OpenCode team.
