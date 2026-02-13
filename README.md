# Sherpa — AI-Guided Code Review Tool

Sherpa pairs a developer reviewing code changes (branch diffs) with an AI guide. Point it at a local Git repository, and the tool analyzes the current branch's changes against its merge base, providing a step-by-step guided review powered by AI.

**Stack:** [Loco.rs](https://loco.rs) (Rust backend), HTMX + FlyonUI (UI), diff2html (client-side diff rendering).

## Prerequisites

- [devbox](https://www.jetify.com/devbox/) — development environment manager
- [just](https://just.systems/) — command runner

## Getting Started

```sh
# Install development tools
devbox shell

# Start the development server
cargo loco start
```

The app starts on **http://localhost:5150**.

### Useful Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api` | Home (JSON) |
| `GET /_health` | Health check |
| `GET /_ping` | Ping |

## Development

```sh
# Run all quality gates
just qa

# Individual commands
just check    # cargo check
just test     # cargo test
just lint     # cargo clippy
just fmt      # cargo fmt
just dev      # cargo loco start
```

## Project Structure

```
├── assets/
│   ├── i18n/           # Internationalization files
│   ├── static/         # Static assets (CSS, JS, images)
│   └── views/          # Tera HTML templates
├── config/
│   ├── development.yaml
│   ├── test.yaml
│   └── production.yaml
├── src/
│   ├── app.rs          # Application hooks and router
│   ├── bin/main.rs     # CLI entry point
│   ├── controllers/    # Request handlers
│   ├── initializers/   # View engine setup
│   └── views/          # Response structs
└── tests/
    └── requests/       # Integration tests
```
