#!/usr/bin/env bash
# tauri-desktop-native-verification: build the real host binary and run its headless probe, so CI
# exercises the same engine resolution, load-guard and settings path the window uses. Set
# SMOLLM_REQUIRE_ENGINE=1 once the fork engine is vendored on the runner (Phase 2+).
source "$(dirname "$0")/_env.sh"

step "cargo test -p smollm-desktop"
cargo test --locked -p smollm-desktop

step "building the desktop host"
cargo build --locked -p smollm-desktop

step "running the host headless: --verify-native"
cargo run --locked -q -p smollm-desktop -- --verify-native
