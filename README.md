# Sherpa — AI-Guided Code Review Tool

Sherpa pairs you with an AI guide to review code changes — on a local Git branch or a **GitHub Pull Request**. Point it at a repository (or paste a PR URL), and Sherpa analyzes the diff — then walks you through a step-by-step guided review with AI-generated explanations, change grouping, and an interactive chat. Sherpa also supports **Review While Building**, where AI coding agents push completed implementation steps for review in real-time while they continue working.

## What Sherpa Does

1. **Analyzes your branch or PR** — detects merge base, extracts diff, counts changed files and lines (works with local Git branches and GitHub Pull Requests)
2. **AI summarizes the changes** — generates an implementation approach overview with key decisions and concerns
3. **Groups changes into review steps** — the AI decides how to organize the diff (by feature, layer, concept — not just file-by-file)
4. **Guides you through each step** — shows the diff (rendered with diff2html), an AI explanation, and how each step relates to the previous one
5. **Chat with the AI** — ask questions at any point, scoped to the current step's context
6. **Track your progress** — validate steps as you go, resume later if interrupted
7. **Review While Building** — AI coding agents can push completed steps in real-time via HTTP API, so you review while they keep working

## Prerequisites

- [devbox](https://www.jetify.com/devbox/) — declarative developer environment (manages Rust, Node.js, and all tooling)
- [direnv](https://direnv.net/) — automatic environment loading (optional but recommended)
- **An AI CLI tool** — either [opencode](https://github.com/opencode-ai/opencode) or [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and accessible in your PATH
- **GitHub CLI** (`gh`) — required only for reviewing GitHub Pull Requests (install from [cli.github.com](https://cli.github.com/))

## Getting Started

```sh
# Allow direnv to auto-load the environment
direnv allow

# Or enter the devbox shell manually
devbox shell

# Start the development server (builds CSS, then starts on port 5150)
just dev
```

Open **http://localhost:5150** and follow the on-screen flow:

1. **Set up AI backend** (`/cli/setup`) — select `opencode` or `claude`, pick models for deep/fast analysis
2. **Point to a repo or PR** (`/repo/analyze`) — enter a local Git repository path (must be on a feature branch) or paste a GitHub PR URL
3. **Wait for analysis** — Sherpa runs background AI calls to generate approach, review plan, and step explanations
4. **Review** — walk through each step, validate changes, chat with the AI

### Configuration

Sherpa stores its config at `~/.sherpa/config.toml`:

```toml
[ai]
selected_cli = "claude"     # or "opencode"
timeout_secs = 240          # per AI call (default: 240)
deep_model = "opus"         # model for complex analysis (optional)
fast_model = "sonnet"       # model for quick tasks (optional)
```

### MCP Server Quick Start

To let AI coding agents (Claude Code, OpenCode) use Sherpa's review-while-building workflow, add the MCP server:

```sh
# Build the MCP server binary
cargo build --bin sherpa-mcp
```

Then register it with your AI tool:

**Claude Code:**
```sh
claude mcp add sherpa --transport stdio --scope user \
  -e SHERPA_URL=http://localhost:5150 \
  -- /path/to/sherpa/target/debug/sherpa-mcp
```

**OpenCode** — add to `opencode.json`:
```json
{
  "mcp": {
    "sherpa": {
      "type": "local",
      "command": ["/path/to/sherpa/target/debug/sherpa-mcp"],
      "environment": { "SHERPA_URL": "http://localhost:5150" }
    }
  }
}
```

See [Review While Building](#review-while-building) for the full setup and workflow details.

### Review State

Review progress is automatically saved to the **SQLite database** (`sherpa_dev.sqlite` in the project root). If you close Sherpa and come back, it will detect the existing review and offer to resume where you left off.

## Review While Building

Sherpa supports a **live review mode** where an AI coding agent (Claude Code, OpenCode, or any tool that can make HTTP calls) pushes completed implementation steps for review while it continues building the next steps. You review in Sherpa's web UI in real-time — no waiting for the agent to finish.

### Quick Start

1. **Start Sherpa** — run `just dev` (or `just up` for full dev environment)
2. **Ask your agent to "review while building"** — when giving it a task in Claude Code or OpenCode, include something like:
   > Implement feature X. Use the Sherpa review-while-building workflow so I can review your progress as you go.
3. **Open the review URL** — the agent creates a session and Sherpa returns a review URL (e.g. `http://localhost:5150/review/{id}/loading`). Open it in your browser.
4. **Review as the agent works** — completed steps appear in real-time with status badges (Planned → Ready for Review → Reviewed). For each step you can:
   - **Read the diff** and the agent's explanation
   - **Chat** with the AI about the step
   - **Validate** to approve the step
   - **Request revision** to send the agent back (optionally blocking it until you're satisfied)
5. **The agent checks your feedback** — periodically (every 2-3 steps) the agent polls for your comments and revision requests, adapting its work accordingly

### How It Works Under the Hood

1. The agent creates a review session with a plan (list of steps)
2. As the agent completes each step, it pushes the diff and explanation to Sherpa
3. You see completed steps appear in Sherpa's UI with status badges
4. You can chat with the AI about each step, request revisions, or block the agent
5. The agent periodically checks for your feedback and adjusts

### Setup: Claude Code or OpenCode

Sherpa provides an MCP server (stdio transport) that gives AI agents native tools for the review workflow. Build it first:

```sh
cargo build --bin sherpa-mcp
```

The resulting binary is at `target/debug/sherpa-mcp` inside your Sherpa clone (e.g. `~/Dev/sherpa/target/debug/sherpa-mcp`). Use `--release` for an optimized build (`target/release/sherpa-mcp`).

The MCP server exposes 6 tools: `create_review_session`, `complete_step`, `push_step`, `check_feedback`, `update_plan`, and `fresh_session`.

Environment variable:
- `SHERPA_URL` — Sherpa server URL (default: `http://localhost:5150`)

#### Claude Code

Add the MCP server using the CLI:

```sh
claude mcp add sherpa --transport stdio --scope user \
  -e SHERPA_URL=http://localhost:5150 \
  -- ~/Dev/sherpa/target/debug/sherpa-mcp
```

Or add it to `.mcp.json` in a project to share with your team:

```json
{
  "mcpServers": {
    "sherpa": {
      "type": "stdio",
      "command": "~/Dev/sherpa/target/debug/sherpa-mcp",
      "env": {
        "SHERPA_URL": "http://localhost:5150"
      }
    }
  }
}
```

#### OpenCode

Add to `opencode.json` (project-level) or `~/.config/opencode/opencode.json` (global):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "sherpa": {
      "type": "local",
      "command": ["~/Dev/sherpa/target/debug/sherpa-mcp"],
      "environment": {
        "SHERPA_URL": "http://localhost:5150"
      }
    }
  }
}
```

### Agent HTTP API

Any tool that can make HTTP calls can integrate directly. The agent API lives under `/api/agent/`:

| Method | Endpoint | Auth | Purpose |
|--------|----------|------|---------|
| POST | `/api/agent/sessions` | None | Create session (returns session_id + agent_token) |
| POST | `/api/agent/sessions/{id}/steps/{n}/complete` | Bearer | Complete a step with diff |
| PUT | `/api/agent/sessions/{id}/steps/{n}` | Bearer | Push intermediate diff |
| GET | `/api/agent/sessions/{id}/feedback` | Bearer | Get reviewer feedback |
| PATCH | `/api/agent/sessions/{id}/plan` | Bearer | Update remaining steps |
| POST | `/api/agent/sessions/{id}/fresh` | Bearer | Replace existing session |

Authentication is per-session: `create` returns an `agent_token`, all other endpoints require `Authorization: Bearer {agent_token}`.

### Reviewer Actions

In the Sherpa web UI, the reviewer can:
- **Validate** a step — marks it as reviewed, advances to the next
- **Request revision** — sets the step to "Needs Revision", optionally blocks the agent
- **Chat** — ask questions scoped to a specific step's diff
- **Track progress** — live progress bar shows completed vs. planned steps

## Development

### Quick Reference

```sh
just dev         # Build CSS + start server (port 5150)
just watch       # Build CSS + cargo-watch (auto-reload on save)
just qa          # Run all quality gates (check + test + clippy)
just test        # cargo nextest run
just lint        # cargo clippy -- -D warnings
just fmt         # cargo fmt
just fmt-check   # cargo fmt --check
just css-build   # Build Tailwind CSS
just css-watch   # Watch & rebuild CSS
just up          # Start all processes (server + CSS watcher)
```

### Process Management

`just up` starts processes via process-compose:
- **server** — `cargo watch` auto-reloading the Loco backend on port 5150
- **css-build** — one-shot Tailwind build (server waits for this)
- **css-watch** — Tailwind CSS watcher, auto-rebuilds on template/CSS changes

### Git Hooks

Managed by prek, configured in .pre-commit-config.yaml:
- **rustfmt** — format check on staged `.rs` files
- **clippy** — lint check on staged `.rs` files
- **nextest** — test suite on staged `.rs` files
- **beads** — issue tracking hooks (pre-commit, pre-push, prepare-commit-msg)

### Quality Gates

All of these must pass before merging:

```sh
cargo check                     # Compilation
cargo nextest run               # Test suite (parallel via nextest)
cargo clippy -- -D warnings     # Lints (warnings are errors)
cargo fmt --check               # Formatting
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend framework | [Loco.rs](https://loco.rs) (Axum + SeaORM) |
| Language | Rust 2021 edition, stable channel |
| Database | SQLite via SeaORM (auto-migrated) |
| Templates | Tera (server-rendered HTML) |
| Frontend interactivity | HTMX (no custom JS framework) |
| UI components | FlyonUI (Tailwind CSS v4 + DaisyUI) |
| Diff rendering | diff2html (client-side JS) |
| Markdown | pulldown-cmark (server-side) |
| AI integration | `opencode` or `claude` CLI (subprocess) + Agent HTTP API |
| MCP server | rmcp (Rust MCP SDK, stdio transport) |
| Dev environment | devbox + just + prek |
| CSS build | `@tailwindcss/cli` v4 |
| Testing | `rstest`, `insta` (snapshots), `serial_test` |

## Project Structure

```
├── assets/
│   ├── css/app.css         # Tailwind CSS source
│   ├── i18n/               # Internationalization (Fluent)
│   ├── static/             # Built CSS, JS, images (gitignored outputs)
│   └── views/              # Tera HTML templates
│       ├── base.html       # Base layout
│       ├── cli/            # AI CLI setup pages
│       ├── home/           # Landing page
│       ├── repo/           # Repository analysis pages
│       └── review/         # Review flow (loading, summary, guide, step)
├── config/
│   ├── development.yaml    # Dev config (SQLite, port 5150)
│   └── test.yaml           # Test config
├── migration/src/          # SeaORM database migrations
├── src/
│   ├── app.rs              # Application hooks and router setup
│   ├── bin/
│   │   ├── main.rs         # CLI entry point
│   │   └── mcp.rs          # MCP server binary
│   ├── controllers/        # HTTP handlers (home, cli, repo, review, agent)
│   │   └── agent.rs        # Agent HTTP API
│   ├── initializers/       # View engine setup
│   ├── models/             # SeaORM models + custom logic
│   │   └── _entities/      # Auto-generated (do not edit)
│   ├── services/           # Business logic
│   │   ├── ai_cli.rs       # AI CLI invocation + prompt building
│   │   ├── background_analysis.rs  # Async AI analysis pipeline
│   │   ├── cli_detection.rs        # CLI availability detection
│   │   ├── config.rs       # ~/.sherpa/config.toml management
│   │   ├── git_analysis.rs # Git operations (merge-base, diff)
│   │   ├── github_pr.rs    # GitHub PR analysis (via gh CLI)
│   │   ├── markdown.rs     # Markdown -> HTML
│   │   └── review_session.rs       # Session struct + live mode
│   └── views/              # Response structs
├── tasks/                  # PRD and task definitions
├── tests/requests/         # Integration tests
├── devbox.json             # Dev environment definition
├── justfile                # Development task definitions
└── Cargo.toml              # Rust dependencies
```

## How It Works

### AI Session Priming

To avoid re-sending the full diff context for every AI call, Sherpa uses a **primed session** pattern:

1. On repo analysis, the full diff is sent to the AI CLI once to create a "primed" session
2. Each subsequent analysis (approach, review plan, step explanations) **forks** from this primed session
3. If priming fails, Sherpa falls back to sending full context with each call

### Model Tiers

AI calls use two tiers, configurable in `~/.sherpa/config.toml`:
- **Deep** — for complex analyses (implementation approach, review plan generation)
- **Fast** — for simpler tasks (step explanations, chat responses, step relations)

### Background Analysis Pipeline

When you submit a repo for analysis, Sherpa spawns background tasks:
1. **Prime session** (or fall back to legacy mode)
2. **Approach analysis** (concurrent with plan)
3. **Review plan generation** (produces step groupings)
4. **Step explanations** (spawned for each step once the plan exists)

The loading page polls for completion status via HTMX and redirects when ready.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
