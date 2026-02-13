# Sherpa development tasks

# Build Tailwind CSS
css-build:
    npm run build

# Watch & rebuild CSS on changes
css-watch:
    npm run css:watch

# Build CSS + start server
dev:
    npm run build && cargo loco start

# Build CSS + cargo-watch (auto-reload on save)
watch:
    npm run build && cargo watch -x 'loco start' -w src -w config -w assets/views

# Build CSS + cargo build
build:
    npm run build && cargo build

# Run tests
test:
    cargo test

# Run cargo check
check:
    cargo check

# Run clippy
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt --check

# Run all quality gates (check + test + clippy)
qa:
    cargo check && cargo test && cargo clippy -- -D warnings

# Start all processes (server + CSS watcher)
up:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill $(jobs -p) 2>/dev/null' EXIT
    npm run css:watch &
    cargo watch -x 'loco start' -w src -w config -w assets/views
