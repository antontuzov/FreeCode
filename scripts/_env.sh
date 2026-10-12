#!/usr/bin/env bash
# Shared helpers for the verify-* scripts. Sourced, never executed directly.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# The workspace pins pnpm in package.json. CI installs it through Corepack; a developer checkout may
# not have it on PATH yet, so fall back to the exact pinned version through npx rather than failing
# with "pnpm: command not found" on a repo that is otherwise fine.
pnpm_cmd() {
  if command -v pnpm >/dev/null 2>&1; then
    pnpm "$@"
  else
    npx --yes "pnpm@$(node -e 'process.stdout.write(require("./package.json").packageManager.split("@")[1])')" "$@"
  fi
}

step() { printf '\n\033[1m▶ %s\033[0m\n' "$*"; }
