#!/usr/bin/env bash
# rust-tauri-precommit-gate: the whole tree must be fmt-clean, warning-free, tested and built
# before anything is called done. Runs in CI on every PR and from .githooks/pre-commit locally.
source "$(dirname "$0")/_env.sh"

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy -D warnings (workspace, all targets)"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test --workspace"
cargo test --workspace

step "frontend format"
pnpm_cmd format:check

step "frontend lint"
pnpm_cmd lint

step "frontend typecheck"
pnpm_cmd typecheck

step "frontend build"
pnpm_cmd build

printf '\n\033[1mrust-tauri-precommit-gate: OK\033[0m\n'
