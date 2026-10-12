#!/usr/bin/env bash
# verify-smollm-studio-natively: the umbrella. If this passes, the Definition of Done's quality
# gate passes; it is the last job in ci.yml and the one to run before pushing.
source "$(dirname "$0")/_env.sh"

for script in verify-catalog.sh verify-precommit.sh verify-tauri-ui.sh verify-desktop-native.sh; do
  step "scripts/$script"
  "./scripts/$script"
done

printf '\n\033[1mverify-smollm-studio-natively: all four gates OK\033[0m\n'
