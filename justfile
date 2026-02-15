# Sherpa development tasks

# Build Tailwind CSS
css-build:
    npm run build

# Watch & rebuild CSS on changes
css-watch:
    npm run css:watch

# Build CSS + start server
dev: css-build
    cargo loco start

# cargo-watch server (auto-reload on save)
server-watch:
    cargo watch -x 'loco start' -w src -w config -w assets/views

# Build CSS + cargo-watch (auto-reload on save)
watch: css-build server-watch

# Build CSS + cargo build
build: css-build
    cargo build

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
qa: check test lint

# Start all processes (server + CSS watcher)
up:
    process-compose up
