# MeshCloud build tasks. On Windows the recipes run under PowerShell; on
# Unix-likes they run under the default POSIX shell.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# List available recipes.
default:
    @just --list

# Format all crates.
fmt:
    cargo fmt --all

# Check formatting without modifying files (CI).
fmt-check:
    cargo fmt --all -- --check

# Run clippy with warnings denied (CI).
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run the full test suite (no external services required).
test:
    cargo test --workspace --all-features

# Run the full test suite with nextest if installed.
test-nextest:
    cargo nextest run --workspace --all-features

# Run integration tests that require Docker (PostgreSQL via testcontainers).
# Serialized because Docker Desktop struggles to start many containers at once.
test-integration:
    cargo test -p mesh-control-plane --features integration-tests -- --ignored --test-threads=1

# Full local verification gate (what CI enforces).
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features

# Sign built Windows executables with a local dev certificate so Application
# Control (Smart App Control / WDAC) does not block them.
sign:
    powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/sign-windows.ps1

# Build, sign, and run tests on Windows (unsigned test binaries are blocked by
# Application Control on some machines). Signing does not trigger a rebuild, so
# the subsequent `cargo test` reuses the signed binaries.
test-windows:
    cargo test --workspace --no-run
    powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/sign-windows.ps1
    cargo test --workspace

# Audit dependencies for security advisories.
audit:
    cargo audit

# Build the Solidity contracts (requires Foundry).
contracts-build:
    forge build --root contracts

# Test the Solidity contracts (requires Foundry and forge-std).
contracts-test:
    forge test --root contracts

# Run a local EVM chain for development.
anvil:
    anvil

# Print control-plane Prometheus metrics.
metrics:
    curl -s http://localhost:8080/api/v1/metrics

# Fuzz protocol and agent-message decoders (requires nightly + cargo-fuzz).
fuzz:
    cargo +nightly fuzz run advertisement -- -max_total_time=30
    cargo +nightly fuzz run agent_message -- -max_total_time=30

# Build release binaries for the host platform.
package:
    cargo build --release --workspace

# Enforce dependency policies (licenses, bans, sources).
deny:
    cargo deny check

# Start the local simulated network (PostgreSQL + control plane + 3 workers).
up:
    docker compose up --build -d

# Stop the local simulated network.
down:
    docker compose down -v

# Show logs from the simulated network.
logs:
    docker compose logs -f

# Run database migrations against the local PostgreSQL (requires sqlx-cli).
migrate:
    cargo sqlx migrate run

# Prepare sqlx offline query data for compile-time checked queries.
prepare:
    cargo sqlx prepare --workspace
