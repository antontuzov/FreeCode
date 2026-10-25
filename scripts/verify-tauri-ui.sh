#!/usr/bin/env bash
# verify-tauri-ui-natively: the shell's contract with the host, checked statically.
#   1. every command the UI invokes exists in the host, and every host command is registered
#   2. every StepKind i18n key is translated in every locale, with matching {{parameters}}
#   3. no component states a colour literally — the theme is a token swap or it is broken
#   4. every token colour is actually legible: 4.5:1 for small text, on every surface it is used on
#   5. every key the UI prints is a key the window answers
#   6. the bundle that ships is the bundle that was type-checked
source "$(dirname "$0")/_env.sh"

IPC=apps/desktop/src/lib/ipc.ts
HOST=apps/desktop/src-tauri/src

step "UI commands vs the host's #[tauri::command] list"
ui_cmds=$(grep -oE 'invoke(<[^>]*>)?\("[a-z_]+"' "$IPC" | grep -oE '"[a-z_]+"' | tr -d '"' | sort -u)
# Every host module can define commands — the engine and chat commands live in chat.rs, not lib.rs —
# and one reached from another module carries `pub`, so both spellings have to count.
host_cmds=$(grep -A3 -h '#\[tauri::command\]' "$HOST"/*.rs \
  | grep -oE '^(pub )?(async )?fn [a-z_]+' | awk '{print $NF}' | sort -u)
registered=$(sed -n '/generate_handler!\[/,/\]/p' "$HOST/lib.rs" | grep -oE '[a-z_]+' | sort -u)

[[ -n "$ui_cmds" ]] || { echo "no invoke() calls found in $IPC — the check itself broke"; exit 1; }
for cmd in $ui_cmds; do
  grep -qx "$cmd" <<<"$host_cmds" || { echo "FAIL the UI calls \"$cmd\" but the host defines no such command"; exit 1; }
  grep -qx "$cmd" <<<"$registered" || { echo "FAIL $cmd is defined but not in generate_handler!, so invoke() rejects it"; exit 1; }
done
for cmd in $host_cmds; do
  grep -qx "$cmd" <<<"$registered" || { echo "FAIL $cmd is a host command that invoke() can never reach"; exit 1; }
done
echo "OK — $(echo "$ui_cmds" | wc -l | tr -d ' ') commands wired both ways: $(echo "$ui_cmds" | tr '\n' ' ')"

step "transcript i18n keys vs StepKind"
cargo run --locked -q -p freecode-core --bin verify-transcript-i18n -- apps/desktop/src/i18n

step "locale catalogs agree, and every lookup is defined"
node scripts/check-i18n.mjs

step "settings search can find every row the pages render"
node scripts/check-settings-index.mjs

step "no hardcoded colours"
pnpm_cmd lint

step "every ink clears WCAG AA on the surfaces it is read against"
node scripts/check-contrast.mjs

step "every advertised key has a handler behind it"
node scripts/check-shortcuts.mjs

step "typecheck and build"
pnpm_cmd typecheck
pnpm_cmd build

printf '\n\033[1mverify-tauri-ui-natively: OK\033[0m\n'
