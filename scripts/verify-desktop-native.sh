#!/usr/bin/env bash
# tauri-desktop-native-verification: build the real host binary and run its headless probe, so CI
# exercises the same engine resolution, load-guard and settings path the window uses. Set
# FREECODE_REQUIRE_ENGINE=1 once the fork engine is vendored on the runner (Phase 2+).
source "$(dirname "$0")/_env.sh"

step "cargo test -p freecode-desktop"
cargo test --locked -p freecode-desktop

step "building the desktop host"
cargo build --locked -p freecode-desktop

step "running the host headless: --verify-native"
cargo run --locked -q -p freecode-desktop -- --verify-native
