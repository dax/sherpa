# Agent Instructions

## Project Overview

**Sherpa** is a single-user local tool that pairs a developer reviewing code changes (branch diffs or GitHub PRs) with an AI guide. The developer points Sherpa at a local Git repository or pastes a GitHub PR URL, and Sherpa analyzes the changes. An AI (via `opencode` or `claude` CLI) provides a project summary, change analysis, and step-by-step guided review. The developer walks through each step, chatting with the AI, and validates changes as they go. Review progress is persisted to SQLite.

## Tech Stack

- **Backend**: [Loco.rs](https://loco.rs) (Rust web framework built on Axum + SeaORM)
- **Frontend**: Server-rendered HTML via Tera templates, HTMX for dynamic updates, FlyonUI (Tailwind CSS v4 + DaisyUI) for components
- **Database**: SQLite (via SeaORM with auto-migration)
- **Diff rendering**: diff2html (client-side JS)
- **AI backends**: `opencode` CLI or `claude` CLI (user-selectable)
- **GitHub PR analysis**: via `gh` CLI (GitHub CLI)
- **Dev environment**: devbox + just + prek
- **Formatter**: rustfmt (`max_width = 100`)
- **CSS build**: Tailwind CSS v4 via `@tailwindcss/cli`, FlyonUI JS copied from node_modules

## Issue Tracking

This project uses **bd** (beads) for issue tracking. Run `bd onboard` to get started.

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --status in_progress  # Claim work
bd close <id>         # Complete work
bd sync               # Sync with git
```

## Architecture

### Source Layout

```
src/
├── app.rs                  # Application hooks, router setup, DB truncation
├── bin/main.rs             # CLI entry point (cargo loco start)
├── controllers/            # HTTP request handlers (page + API routes)
│   ├── home.rs             # Landing page (GET /)
│   ├── cli.rs              # AI CLI setup/selection (GET /cli/setup, POST /cli/select)
│   ├── repo.rs             # Repository analysis (GET /repo/analyze, POST /repo/analyze)
│   └── review.rs           # Review flow: loading -> summary -> guide -> step (main controller)
├── services/               # Business logic (no HTTP concerns)
│   ├── ai_cli.rs           # AI CLI invocation, prompt building, session priming/forking
│   ├── background_analysis.rs  # Async background AI analysis (approach, plan, step explanations)
│   ├── cli_detection.rs    # Detect available AI CLIs (opencode/claude), list models
│   ├── config.rs           # SherpaConfig (TOML at ~/.sherpa/config.toml)
│   ├── git_analysis.rs     # Git operations (merge-base, diff, changed files)
│   ├── github_pr.rs        # GitHub PR analysis (via gh CLI)
│   ├── markdown.rs         # Markdown -> HTML conversion (pulldown-cmark)
│   └── review_session.rs   # ReviewSession struct, persistence, review plan, live mode
├── models/                 # SeaORM models (DB layer)
│   ├── review_sessions.rs  # Review session DB operations
│   ├── ai_analyses.rs      # Cached AI analysis results + failure tracking
│   ├── chat_messages.rs    # Chat message storage
│   └── _entities/          # Auto-generated SeaORM entities
├── initializers/
│   └── view_engine.rs      # Tera view engine setup with i18n (fluent)
├── views/                  # Response structs
├── data/                   # Data module
└── tasks/                  # Loco tasks
```

### Templates

```
assets/views/
├── base.html               # Base layout (HTMX, FlyonUI, diff2html assets)
├── home/
│   ├── index.html          # Landing page
│   └── _greeting.html      # HTMX partial
├── cli/
│   ├── setup.html          # AI CLI selection page
│   ├── _success.html       # Success partial after CLI selection
│   ├── _error.html         # Error partial
│   └── _model_selects.html # Model dropdown partials (loaded via HTMX)
├── repo/
│   ├── analyze.html        # Repository path input page
│   ├── _success.html       # Success partial
│   ├── _error.html         # Error partial
│   └── _resume_prompt.html # Resume/fresh review prompt
└── review/
    ├── loading.html        # Loading screen during background analysis
    ├── summary.html        # Project/change summary with AI sections
    ├── guide.html          # Review plan overview (all steps listed)
    ├── step.html           # Individual review step (diff + explanation + chat)
    └── _*.html             # Various HTMX partials for dynamic updates
```

### Key Data Flow

1. **CLI Setup** (`/cli/setup`): User selects AI backend -> saved to `~/.sherpa/config.toml`
2. **Repo Analysis** (`/repo/analyze`): User submits repo path -> git analysis -> ReviewSession created -> background AI analyses spawned -> redirect to loading page
3. **Loading** (`/review/{id}/loading`): Polls `/review/{id}/status` via HTMX every 2s -> redirects to summary when ready
4. **Summary** (`/review/{id}/summary`): Shows AI-generated approach + metrics + chat -> user clicks "Start Review"
5. **Guide Start** (`/review/{id}/guide/start`): AI generates review plan (JSON with steps) -> saved to DB
6. **Step Review** (`/review/{id}/guide/step/{n}`): Shows diff (via diff2html), AI explanation, relation to previous step, step-scoped chat
7. **Validation** (`/review/{id}/guide/step/{n}/validate`): Marks step validated -> advances to next -> redirects to summary when all done

### AI CLI Integration

Sherpa uses a **primed session** pattern to avoid re-sending the full diff context for every AI call:

1. **Prime**: Send full diff context once, get a session ID back
2. **Fork**: Each subsequent analysis (approach, plan, step explanations) forks from the primed session
3. **Fallback**: If priming fails, falls back to legacy mode (full context per call)

AI calls use two model tiers:
- `ModelTier::Deep` -- for complex analyses (approach, review plan)
- `ModelTier::Fast` -- for simpler tasks (step explanations, chat, relations)

Model tiers are configurable per-CLI in the config.

### Database Schema

Three tables (SQLite, auto-migrated):
- `review_sessions` -- session state (repo path, branch, diff, review plan JSON, validated steps, primed session ID)
- `ai_analyses` -- cached AI outputs keyed by (session_id, analysis_type, step_number), with status (success/failure) tracking
- `chat_messages` -- chat history keyed by (session_id, step_number)

### State Persistence

Review state is saved to the **SQLite database** (primary and only storage for all session data). Updated on every mutation (step validation, chat message, plan generation).

## Route Reference

### Page Routes (HTML)

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/` | `home::index` | Landing page |
| GET | `/cli/setup` | `cli::cli_setup` | AI CLI selection |
| POST | `/cli/select` | `cli::cli_select` | Save CLI selection |
| GET | `/cli/models?cli=X` | `cli::cli_models` | Fetch model list (HTMX) |
| GET | `/repo/analyze` | `repo::analyze_page` | Repo path input |
| POST | `/repo/analyze` | `repo::analyze_submit` | Submit repo for analysis |
| POST | `/repo/resume/{id}` | `repo::resume_submit` | Resume existing review |
| POST | `/repo/fresh` | `repo::fresh_submit` | Start fresh review |
| GET | `/review/{id}/loading` | `review::loading_page` | Analysis loading screen |
| GET | `/review/{id}/status` | `review::analysis_status` | Analysis status (HTMX poll) |
| GET | `/review/{id}/summary` | `review::summary_page` | Review summary |
| GET | `/review/{id}/summary/approach` | `review::summary_approach` | Approach section (HTMX) |
| POST | `/review/{id}/summary/chat` | `review::summary_chat` | Summary chat |
| POST | `/review/{id}/guide/start` | `review::guide_start` | Generate review plan |
| GET | `/review/{id}/guide` | `review::guide_page` | Review plan overview |
| GET | `/review/{id}/guide/step/{n}` | `review::step_page` | Step review page |
| GET | `/review/{id}/guide/step/{n}/explanation` | `review::step_explanation` | Step explanation (HTMX) |
| GET | `/review/{id}/guide/step/{n}/relation` | `review::step_relation` | Step relation (HTMX) |
| POST | `/review/{id}/guide/step/{n}/chat` | `review::step_chat` | Step chat |
| POST | `/review/{id}/guide/step/{n}/validate` | `review::step_validate` | Validate step |

### API Routes (JSON)

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/api/cli/status` | `cli::cli_status` | CLI detection status |

## Quality Gates

These must pass before any code is merged:

```bash
cargo check                     # Compilation
cargo nextest run               # Test suite (parallel via nextest)
cargo clippy -- -D warnings     # Lints (warnings = errors)
cargo fmt --check               # Formatting check

# All at once:
just qa
```

For UI changes, also verify in browser (see "Accessing the Application" below).

## Accessing the Application

The server port is configured via the `PORT` env var (set per-branch in `.local_envrc` by worktrunk, defaults to `5150`):

```
http://localhost:${PORT:-5150}
```

To check your current port: `echo $PORT` (requires direnv to have loaded `.envrc` → `.local_envrc`).

## Development Commands

```bash
just dev           # Build CSS + start server (one-shot, no auto-reload)
just watch         # Build CSS + cargo-watch (auto-reload)
just server-watch  # cargo-watch server only (no CSS build)
just test          # cargo nextest run
just lint          # cargo clippy
just fmt           # cargo fmt
just qa            # check + test + clippy
just css-build     # Build Tailwind CSS
just css-watch     # Watch & rebuild CSS
```

## Process Management

All dev processes are orchestrated via **process-compose**, which delegates to `just` recipes (single source of truth for commands).

```bash
just up            # Start all processes (server + CSS build + CSS watcher)
```

Under the hood, `just up` runs `process-compose up`, which starts:
- **server** — `just server-watch` (cargo-watch auto-reload, waits for css-build)
- **css-build** — `just css-build` (one-shot Tailwind build, runs first)
- **css-watch** — `just css-watch` (Tailwind watcher, starts after css-build)

### Stopping / Restarting

```bash
# Stop all processes (from another terminal in the same direnv)
process-compose down -p ${PROCESS_COMPOSE_PORT:-9999}

# Restart a single process (e.g. after config change)
process-compose restart server -p ${PROCESS_COMPOSE_PORT:-9999}

# Or just Ctrl-C the `just up` terminal to stop everything
```

## Conventions

### Rust

- Edition 2021, stable channel
- rustfmt: `max_width = 100`
- All controllers expose `page_routes()` and `api_routes()` functions
- Services contain business logic, controllers handle HTTP
- Use `tracing::info!`/`warn!`/`error!` for logging
- Errors: custom enums implementing `Display` + `Error` (no `anyhow`/`thiserror`)
- Tests: in-module `#[cfg(test)] mod tests`, using `rstest`, `insta` (snapshots), and `serial_test`
- SeaORM entities in `models/_entities/` (auto-generated, do not edit)
- Custom model logic in `models/{name}.rs`

### Frontend

- Tera templates extend `base.html`
- HTMX for all dynamic interactions (no custom JS beyond diff2html initialization)
- FlyonUI component classes for all UI elements
- Partials prefixed with `_` (e.g., `_section_content.html`)
- Dark theme: `data-theme="dark"` on root element
- AI-generated content rendered as Markdown -> HTML server-side via `pulldown-cmark`

### File Organization

- New controller? Add to `src/controllers/mod.rs` and register routes in `src/app.rs`
- New service? Add to `src/services/mod.rs`
- New model? Add to `src/models/mod.rs` and create migration in `migration/src/`
- New template? Place in `assets/views/{controller_name}/`
- Static assets? Place in `assets/static/` (served at `/static/`)

### Documentation Maintenance

After every code change, check whether `README.md` or `AGENTS.md` need updating:

- **New route, controller, or service?** → Update the route table, source layout, and architecture sections in both files
- **Changed data flow or persistence?** → Update the "Key Data Flow" and "State Persistence" sections
- **New dependency or tool?** → Update the "Tech Stack" table and "Prerequisites" in README.md
- **Changed CLI commands or config?** → Update "Getting Started", "Configuration", and "Development Commands" sections
- **New MCP tool or API endpoint?** → Update the MCP and Agent HTTP API sections in README.md

**Rule**: If your change would make existing documentation inaccurate, update the docs in the same commit.

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** -- Create beads issues for anything that needs follow-up
2. **Run quality gates** (if code changed) -- `just qa`
3. **Update issue status** -- Close finished work, update in-progress items
4. **PUSH TO REMOTE** -- This is MANDATORY:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** -- Clear stashes, prune remote branches
6. **Verify** -- All changes committed AND pushed
7. **Hand off** -- Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing -- that leaves work stranded locally
- NEVER say "ready to push when you are" -- YOU must push
- If push fails, resolve and retry until it succeeds

