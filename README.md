# Sherpa — AI-Guided Code Review Tool

Sherpa pairs you with an AI guide to review code changes on any local Git branch. Point it at a repository, and Sherpa analyzes the current branch's diff against its merge base — then walks you through a step-by-step guided review with AI-generated explanations, change grouping, and an interactive chat.

## What Sherpa Does

1. **Analyzes your branch** — detects merge base, extracts diff, counts changed files and lines
2. **AI summarizes the changes** — generates an implementation approach overview with key decisions and concerns
3. **Groups changes into review steps** — the AI decides how to organize the diff (by feature, layer, concept — not just file-by-file)
4. **Guides you through each step** — shows the diff (rendered with diff2html), an AI explanation, and how each step relates to the previous one
5. **Chat with the AI** — ask questions at any point, scoped to the current step's context
6. **Track your progress** — validate steps as you go, resume later if interrupted

## Prerequisites

- [devbox](https://www.jetify.com/devbox/) — declarative developer environment (manages Rust, Node.js, and all tooling)
- [direnv](https://direnv.net/) — automatic environment loading (optional but recommended)
- **An AI CLI tool** — either [opencode](https://github.com/opencode-ai/opencode) or [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and accessible in your PATH

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
2. **Point to a repo** (`/repo/analyze`) — enter the path to a local Git repository (must be on a feature branch, not main/master)
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

### Review State

Review progress is automatically saved to:
- **SQLite database** — `sherpa_dev.sqlite` in the project root
- **JSON files** — `~/.sherpa/sessions/{id}.json` and `{repo}/.sherpa/review-{branch}.json`

If you close Sherpa and come back, it will detect the existing review and offer to resume where you left off.

## Development

### Quick Reference

```sh
just dev         # Build CSS + start server (port 5150)
just watch       # Build CSS + cargo-watch (auto-reload on save)
just qa          # Run all quality gates (check + test + clippy)
just test        # cargo test
just lint        # cargo clippy -- -D warnings
just fmt         # cargo fmt
just fmt-check   # cargo fmt --check
just css-build   # Build Tailwind CSS
just css-watch   # Watch & rebuild CSS
just up                           # Start all processes (server + CSS watcher)
```

### Process Management

`just up` starts two processes via process-compose:
- **server** — `cargo watch` auto-reloading the Loco backend on port 5150
- **css-watch** — Tailwind CSS watcher, auto-rebuilds on template/CSS changes

### Git Hooks

Managed by prek, configured in .pre-commit-config.yaml:
- **rustfmt** — format check on staged `.rs` files
- **clippy** — lint check on staged `.rs` files
- **beads** — issue tracking hooks (pre-commit, pre-push, prepare-commit-msg)

### Quality Gates

All of these must pass before merging:

```sh
cargo check                     # Compilation
cargo test                      # Test suite
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
| AI integration | `opencode` or `claude` CLI (subprocess) |
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
│   ├── bin/main.rs         # CLI entry point
│   ├── controllers/        # HTTP handlers (home, cli, repo, review)
│   ├── initializers/       # View engine setup
│   ├── models/             # SeaORM models + custom logic
│   │   └── _entities/      # Auto-generated (do not edit)
│   ├── services/           # Business logic
│   │   ├── ai_cli.rs       # AI CLI invocation + prompt building
│   │   ├── background_analysis.rs  # Async AI analysis pipeline
│   │   ├── cli_detection.rs        # CLI availability detection
│   │   ├── config.rs       # ~/.sherpa/config.toml management
│   │   ├── git_analysis.rs # Git operations (merge-base, diff)
│   │   ├── markdown.rs     # Markdown -> HTML
│   │   └── review_session.rs       # Session struct + file persistence
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
