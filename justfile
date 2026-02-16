# Sherpa development tasks

# Install frontend dependencies
vendor-install:
    npm install

# Build Tailwind CSS + copy vendor assets
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

# Run tests (via cargo-nextest for parallel execution)
test:
    cargo nextest run

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
qa: fmt check test lint

# Start all processes (server + CSS watcher)
up:
    process-compose -p ${PROCESS_COMPOSE_PORT:-9998} up

@start service:
    process-compose -p ${PROCESS_COMPOSE_PORT:-9998} process start {{ service }}

@stop service:
    process-compose -p ${PROCESS_COMPOSE_PORT:-9998} process stop {{ service }}

@logs service:
    process-compose -p ${PROCESS_COMPOSE_PORT:-9998} process logs -n 100 -f {{ service }}
