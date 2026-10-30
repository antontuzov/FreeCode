#!/usr/bin/env bash
# tauri-desktop-native-verification: build the real host binary, so CI exercises the settings path
# and command surface the window uses rather than only the crates underneath it.
source "$(dirname "$0")/_env.sh"

step "cargo test -p freecode-desktop"
cargo test --locked -p freecode-desktop

step "building the desktop host"
cargo build --locked -p freecode-desktop
