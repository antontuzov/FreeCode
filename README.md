# FreeCode

A desktop coding agent that runs on **free models**. FreeCode ships pointed at the free tier
[OpenCode](https://opencode.ai) serves — a real agent loop, no account, no API key, no bill — with
Big Pickle as the model it starts on.

There is no local model in this build: no weights to download, no server to start, no engine to
install. A third route — any OpenAI-compatible provider — is available as an explicit opt-in.

Tauri v2 shell, Rust host, React interface, and a three-column workbench: transcript in the middle,
workspace on the left, the run's report on the right. Every tool call leaves a row, every refusal
names its reason in the language the interface is read in, and every number in the inspector is one
the service or the host actually measured.

## The two routes

| Route | Who answers | What it needs |
| --- | --- | --- |
| **Free — OpenCode** (default) | The free models OpenCode publishes, Big Pickle first | OpenCode installed on this Mac; FreeCode starts its service |
| **Remote** (optional) | Any OpenAI-compatible endpoint | An HTTPS base URL and a key, both chosen in Settings |

The composer's model pill picks the route for the next turn, and the status strip names the one that
answered. "Remote" never falls back to the free tier: a route that is not available refuses the turn
and says what is missing.

## Free models: OpenCode's tier

OpenCode's free models are served only to OpenCode's own runtime. FreeCode is that runtime's client,
not a way around it: the turn is relayed through the daemon the OpenCode app keeps on loopback, using
the handshake its gateway expects. Nothing is scraped, no credential is invented, and the app does not
bundle OpenCode — it uses the copy already on the Mac, and runs that copy's `service start` when its
daemon is not up yet. A machine without OpenCode gets a refusal that names the one download it needs,
not a set of free rows that can never answer.

The list is what `opencode models` prints under its own provider — read from the CLI the installed app
ships (v2.2.0) on 2026-10-29. The service hands out the list, so it moves without an app update, and a
release can add rows the pinned registry does not carry yet.

| Model id | Label | Carrier |
| --- | --- | --- |
| `big-pickle` | Big Pickle | FreeCode — the default this build starts on |
| `glm-5.3-flash-free` | GLM-5.3-Flash | GLM |
| `deepseek-v4.1-flash-free` | DeepSeek V4.1 Flash | DeepSeek |
| `beam-free` | Beam | Beam |
| `flint-40b-free` | Flint 40B | Flint |
| `mimo-v2.6-flash-free` | MiMo-V2.6-Flash | MiMo |
| `nemotron-3.5-lightning-free` | Nemotron 3.5 Lightning | NVIDIA |
| `nemotron-3.5-super-vl-free` | Nemotron 3.5 Super VL | NVIDIA |

**The default is free.** A settings document that never picked a model — and had not already chosen a
remote provider — is migrated onto Big Pickle once; a new conversation opens on it, and a free turn
needs neither the Privacy switch nor a key.

**One route, and no key.** That service is the only thing that answers a free row. Earlier builds also
carried an optional OpenRouter key as a fallback and walked a failed turn down its own list; that route
is gone, because its ids were this app's guess at a mirror of OpenCode's list rather than a route
OpenCode publishes, and a row naming DeepSeek or NVIDIA while asking a third party for it said
something about those makers that was not true. So **when the service is not running**, every free row
is listed and not pickable, and a turn sent anyway is refused with that reason — never moved to a model
nobody chose.

**Free means cloud.** A free-model prompt leaves this Mac and is answered by OpenCode's provider,
under their terms. The optional remote route is the only other way to answer a prompt.

## What the agent does

Every prompt is an agent prompt. The model reads and searches the project, creates files, edits them
and applies whole-file patches, and narrates each round in the transcript. Each change lands in a
review panel with its diff, line numbers and a find control, and a press on its file row hands the
page to the desktop's own handler.

On the free route the relay sees a write only once the service reports it, after the file has changed,
so it holds no before-image and the review panel refuses a revert it cannot prove rather than
restoring a state it does not have.

- **The workspace is bounded.** Reads and writes are confined to the project folder chosen in
  Settings → Agent → Project. A folder that has since moved refuses the run by name rather than
  falling back somewhere else.
- **On the free route** the OpenCode session carries its own read, search, edit and patch tools. Every
  path it would write is checked against the same sandbox, and the permission request it raises is
  answered synchronously: inside the folder under the approval dial's policy, refused outside it.
  The shell, web fetch and browser tools are denied for a free-model run whatever the switches say —
  those belong to the remote route, which prices them through their own controls.
- **Plan only** turns the same run into a proposer: read-only calls, a plan-shaped answer, and a Plan
  mark so it cannot be read as a change that already happened.
- **Nothing asks mid-run.** The approval dial is read when the prompt is sent; a call it refuses comes
  back as a blocked row naming the policy. There is no prompt a run could wait on in this build.

## What this build does not do

Said plainly, because the interface says it too — a control that is not built is disabled and names
the reason on hover:

- Sixteen of the thirty declared tools have executors, and `spawn_subagent` is answered by the run loop
  rather than an executor. The rest are declared so the model can ask; a call to one is answered with a
  row that says it was not run. The runners and the three `git` reads are real: they execute the
  workspace's own manifest through argv, and they are offered only while Terminal access is on.
- Nothing produces an artifact yet, so that inspector group is the one still scripted under
  `?fixture=1`.
- You cannot attach a file to a prompt. The composer's add control is dead until there is a retrieval
  path to put it in.
- No telemetry exists to switch off: nothing in this build collects or sends usage data.

## Remote providers (optional)

Settings → Model → Remote provider can send prompts to an OpenAI-compatible endpoint instead of the
free tier. It is opt-in twice: the Privacy switch *Allow remote providers* must be
on, and the source must be set to Remote. The key is stored in the macOS Keychain, never in the
settings file, IPC answers, logs or process arguments, and is sent as `Authorization: Bearer` only
while a request is being built.

The base URL must be `https://…`; plain `http://` is accepted only for a loopback address. The app
appends `/v1/chat/completions`. No sampler values or reasoning-effort field are sent — an arbitrary
endpoint has neither, so its own defaults apply.

## Requirements

- macOS on Apple Silicon. CI and the native builds were verified here; the lock records Intel, Windows
  and Linux targets, but nothing has been run on them.
- The [OpenCode](https://opencode.ai) app for the free route. FreeCode starts its service itself and
  says when OpenCode is not installed.
- Rust stable with `rustfmt` and `clippy`.
- Node 22 or newer, and pnpm 11 through corepack (`packageManager` is pinned in the root manifest).

## Getting started

```sh
pnpm install
pnpm app:dev
```

Install OpenCode once; the free tier is the default and the first prompt is answered on Big Pickle —
the app starts the service for you. To build the app bundle: `pnpm app:build`.

## Where things live

| What | Where |
| --- | --- |
| Settings | `~/Library/Application Support/freecode/settings.json` (platform standard elsewhere) |
| Remote provider key | macOS Keychain, service `studio.freecode.desktop.remote-provider` — the only key this app reads |
| Filed conversations | `sessions/` beside the settings file, one JSON file each |
| Deleted conversations | `sessions/trash/`, until Settings → Storage erases one for good |
| Server pid registry | `servers.pid` beside it |

A checkout reads its data from the repo root. An installed `.app` uses the directory it owns;
`FREECODE_ROOT` overrides the root and is honoured strictly, so a wrong override fails loudly rather
than verifying the wrong tree.

## Checks

`./scripts/verify-precommit.sh` is the gate — fmt, clippy at `-D warnings`, the whole Rust test suite,
frontend format, lint, typecheck, store tests and build. It runs in CI on every PR and from
`.githooks/pre-commit`.

Alongside it, `verify-tauri-ui.sh` boots the real window, `verify-desktop-native.sh` builds and tests
the host, and `verify-studio-natively.sh` runs the whole set. `pnpm verify:all` is the long path.

Four of the checks exist because a claim that is not enforced rots within a commit:

- `check-i18n.mjs` — every key defined in both catalogs, and every literal lookup defined. The Rust
  side of the same promise is `verify-transcript-i18n`: a row's label, a tool's one-line result, a
  notice, a refusal's three sentences.
- `check-contrast.mjs` — every grey and every style ink at 4.5:1 on every surface each of the eight
  styles paints, in both modes.
- `check-shortcuts.mjs` — every binding in `lib/shortcuts.ts` answered by a handler, and every hint
  that names a key reading its label from the table.
- `check-settings-index.mjs` — the search index matching the rows each Settings page actually renders.

## Looking at the UI without clicking

The native window takes no injected clicks on a developer machine, and several states are one press
deep. The dev URL accepts seeds that no shipped control ever sets:

| Seed | Opens |
| --- | --- |
| `?fixture=1` | a scripted run, so the transcript and inspector render with nothing to show yet |
| `?plan=1` | the run's plan panel, opened over the fixture's list |
| `?collapsed=<groups>` | the inspector's lower groups |
| `?settings=<page>`, `?q=<query>` | a Settings page, and the search field |
| `?review=1`, `?find=<text>`, `?scope=all`, `?outline=1` | the review panel, its find field, its scope and its fold |
| `?ask=<prompt>` | a real agent run, started from the URL |
| `?extensions=1` | the Extensions inventory |

`pnpm app:dev:fixture`, `app:dev:settings` and `app:dev:chat` are the three pre-wired ones.

## Shipping

`pnpm app:build` produces an unsigned `.app` and a `.dmg` under `target/release/bundle/`. A second
launch never starts a second app: the new process hands over and the open window comes forward.

Two release steps are not wired yet, and this build makes no claim to have them:

- **Signing and notarisation.** The bundle is ad-hoc signed, so a copy downloaded from the internet is
  blocked by Gatekeeper until it is allowed in System Settings → Privacy & Security.
- **The updater.** It needs a published update endpoint and a signing key pair whose public half ships
  in the app. Neither exists here, so the app does not check for updates.

## Layout

```
crates/freecode-config    settings document, privacy and approval policy
crates/freecode-models    the retired local route's catalog contract, kept in-tree, not wired in
crates/freecode-llama     the fork's remote streaming types, kept in-tree, not wired in
crates/freecode-tools     the tool registry, the sandbox, the two web calls
crates/freecode-core      the transcript model, the agent loop, approvals
apps/desktop              the Tauri v2 host, the OpenCode relay and the React interface
docs/reference-ui         the screenshots the interface is matched against
docs/KNOWN_ISSUES.md      every model limit the code defends against, and where the defence is
```

## Third-party

- **OpenCode** — the free tier and the loopback service the free route is relayed through. FreeCode is
  not affiliated with OpenCode; the app uses the copy the user installed and its published free list.
- **Simple Icons (CC0)** — the path data behind the seven marks the app wears: three site signs in the
  inspector (Hugging Face, GitHub, Wikipedia) and four maker signs on the model rows (DeepSeek, NVIDIA,
  Xiaomi, MiniMax). The signs remain their owners' trademarks; only the icon geometry is shared.
- **KaTeX** fonts and stylesheet, bundled with the frontend build.

FreeCode is MIT licensed; see [`LICENSE`](LICENSE).
