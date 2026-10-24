# OpenFree

A local-first desktop chat and coding agent for [PrismML's ternary Bonsai models](https://huggingface.co/prism-ml),
with a free-model tier answered by the OpenCode app already on this Mac. Tauri v2 shell, Rust host, one
supervised `llama-server` on loopback, and weights that stay on your disk.

The interface is modelled on a three-column AI workbench — transcript in the middle, workspace on the left,
a run inspector on the right — and it is built to say what it did: every tool call leaves a row, every
refusal names its reason in the language you read the app in, and every number in the inspector is one the
engine or the host actually measured.

## What it does

- **Chats with a local model.** Streams tokens and reasoning, renders the answer as markdown with real
  math (KaTeX) and code fences with a copy control, and keeps the reasoning of past turns visible while
  replaying only the answers, so the fork's prompt cache keeps hitting. A remote OpenAI-compatible
  provider can answer instead, behind an explicit source choice, the Privacy switch and a key held in the
  macOS Keychain.
- **Runs a coding loop.** Every prompt this app sends is an agent prompt: the model reads, searches,
  creates files, edits them and applies a multi-file patch; with the Privacy switch on it looks things
  up on the web, with Terminal access on it runs one command, and with Subagents on it may hand
  one self-contained question to a read-only second model run. That is what lets it lay out a small app
  from nothing rather than only patch one. The approval dial's **Plan only** mode turns the same run into
  a proposer: it explores with the read-only calls and answers with a numbered plan instead of touching
  the workspace. Each round narrates itself in the transcript; each write lands
  in a review panel with its diff, its line numbers, a find control inside it, and a press on its file row
  that hands the page to the desktop's own handler.
- **Files conversations on this machine.** The sidebar searches them, reopens them, and moves deleted
  ones to a Trash that Settings → Storage can put back.
- **Reports the run.** Token rates, prompt-cache reuse, the wait, the pages read, the files opened, the
  files changed, and the capabilities the run called on — all derived from the transcript's own rows, so
  the panel and the transcript cannot tell two stories.
- **Speaks two languages and paints eight ways.** English and Russian, light and dark, eight interface
  styles that repaint the whole window including the sidebar, and a Dock tile drawn from the app's own
  mascot set. A settings file that still names a catalog this build dropped opens in English rather
  than failing to open.

## What this build does not do

Said plainly, because the interface says it too — a control that is not built is disabled and names the
reason on hover:

- Sixteen of the thirty declared tools have executors, and one more — `spawn_subagent` — is answered by
  the run loop rather than an executor. `delete_path`, `move_path`, `format_code`, `codebase_search`,
  `git_stage`, `git_commit`, `git_push`, `http_request`, `install_extension`, `mcp_call`,
  `create_artifact`, `ask_user` and `wait_for_port` are declared so the model can ask, and a call to one
  is answered with a row that says it was not run. The runners and the three `git` reads are real: they
  execute the workspace's own manifest through argv, and they are offered only while Terminal access is
  on (the `git` reads need no switch — they change nothing and run a fixed command).
- Nothing produces an artifact yet, so that inspector group is the one still scripted under
  `?fixture=1`. The Subagents group is fed by the run's own delegation rows; a real run leaves it empty
  while the Subagents switch is off.
- You cannot attach a file to a prompt. The composer's add control is deliberately dead until there is a
  retrieval path to put it in.
- The supervised server is **not authenticated**. It binds `127.0.0.1` on a port the host picks, and any
  process on this machine that finds the port can use the model. Treat that as the current boundary, not
  an oversight.
- No telemetry exists to switch off: nothing in this build collects or sends usage data.

## How a model runs

`engines/lock.toml` pins one release of the PrismML fork of llama.cpp (`prism-b10787-73bfe6c` today) and
records the sha256 of every release asset the app can install. The install refuses to unpack an archive
whose digest does not match, so a substituted asset cannot reach the loader. The engine is downloaded, not
vendored.

The host then launches `llama-server --host 127.0.0.1 --port <free>`, supervises it, and writes the pid
beside `settings.json` so a crash cannot leave a several-gigabyte server holding weights nobody owns. A
`SIGTERM` that reaches the app kills the server on the way out. Stop in the composer stops the reply, not
the engine; a separate control releases the weights.

`catalog/models.toml` is the same kind of contract for weights: sizes and sha256 values read from the
Hugging Face tree API and the repos' own `SHA256SUMS`, context lengths read from the GGUF headers, and the
known issues quoted per pack. Three models are catalogued today — Bonsai 2 27B, Bonsai 2 4B and Ternary
Bonsai 1.7B — and the download happens in the app.

### Remote providers

Settings → Model → Remote provider can send prompts to an OpenAI-compatible endpoint instead of the local
engine. This is opt-in twice: the Privacy switch *Allow remote providers* must be on, and the source must
be set to Remote. With either missing, a prompt is refused with the setting named — it is never answered by
the local engine as a fallback, so "Remote" cannot silently mean "local anyway".

- **The key never enters the settings document.** It is stored in the macOS Keychain (service
  `studio.smollm.desktop.remote-provider`, account `api-key`), read only when a request is built, held in
  memory for that turn, and sent as `Authorization: Bearer`. The settings file, IPC answers, logs and the
  process arguments never contain it; the UI can only ask whether one is saved.
- **HTTPS, or loopback.** The base URL must be `https://…`; plain `http://` is accepted only for a
  loopback address, which is what makes a local provider testable without a certificate. Credentials
  pasted inside the URL are refused, so they cannot end up saved as part of the address.
- **What is sent.** The messages (including an attached image, if the provider's model accepts one), the
  model id, the reply budget, the tool schemas, and `stream_options.include_usage`. The app appends
  `/v1/chat/completions` to the base URL.
- **What is not sent.** No sampler values and no reasoning-effort field: those belong to the pinned fork's
  template, and an arbitrary endpoint has neither. The provider's own defaults apply.
- **What is not supported.** Non-OpenAI auth schemes (an `api-key` header, signed requests), provider-
  specific request fields, and counting a prompt's tokens before sending it. The meter reads the usage the
  provider returns; a prompt too large for the declared window is reported by the provider, not predicted
  here. The declared context window also caps the reply budget at half its size.

## What the agent may reach

- **The workspace.** Reads (`read_file`, `list_dir`, `glob`, `grep_search`) and the two writes
  (`write_file`, which only ever creates a file that is not there, and `edit_file`) are bounded to the
  workspace root the app reports on its Extensions page. That root is the project folder chosen in
  Settings → Agent → Project, opened with the desktop's own folder dialog; until one is chosen it is
  the checkout (development) or the app's data directory (installed). A chosen folder that has since
  moved refuses the run by name instead of falling back to either.
- **The web, only when you say so.** `web_search` and `fetch_url` are blocked while
  Settings → Agent → Privacy is off, which is the default. When it is on, search goes to Wikipedia's
  keyless REST endpoint — one provider, named in the text the model reads — and `fetch_url` reads a
  public page back with three guards: http/https only, loopback and private ranges refused *after* the
  hostname resolves and again on every redirect, and the result capped both on the wire and in
  characters. DNS rebinding remains the residual risk of reading arbitrary URLs at all, which is why the
  whole module sits behind that one switch.
- **Approvals.** The mode the next run is built with — the composer's pill and Settings → Agent both
  edit the same value, which the host re-reads when a prompt is sent. A call it refuses comes back as a
  blocked row naming the policy. Nothing asks: this build has no prompt a run could wait on, so the mode
  named "Ask every time" runs nothing. **Plan only** is a mode rather than a refusal alone: an agent
  run under it is offered the read-only calls, given a contract that asks for a plan, and its answer
  carries a Plan mark so it cannot be read as a change that already happened. The Extensions page
  computes each tool's answer with the same engine the loop uses mid-run, so the page cannot tell a
  friendlier story than the agent acts.

## Where things live

| What | Where |
| --- | --- |
| Settings | `~/Library/Application Support/smollm-studio/settings.json` (platform standard elsewhere) |
| Remote provider key | macOS Keychain, service `studio.smollm.desktop.remote-provider`, account `api-key` |
| Filed conversations | `sessions/` beside it, one JSON file each |
| Deleted conversations | `sessions/trash/`, until Settings → Storage erases one for good |
| Server pid registry | `servers.pid` beside it |
| Catalog and engine contract | `catalog/models.toml`, `engines/lock.toml` |
| Engine | `engines/<pinned-tag>/<target>/llama-server` |
| Weights | `models/*.gguf` |

A checkout reads all of that from the repo root. An installed `.app` reads the two contract files from its
own bundle — the code signature covers everything inside it, so downloading a 463 MB weight pack into the
bundle is not an option, and `/Applications` is not writable at all — and mirrors them into the directory
the app owns, which then becomes the root. `SMOLLM_STUDIO_ROOT` overrides both, and is honoured strictly:
a wrong override fails loudly rather than quietly verifying the wrong tree.

## Requirements

- macOS on Apple Silicon. That is where CI, Metal, the pinned engine asset and the hardware detection in
  the status card were all built and verified; the lock records Intel, Windows and Linux targets, but
  nothing here has been run on them.
- Rust stable with `rustfmt` and `clippy`.
- Node 22 or newer, and pnpm 11 through corepack (`packageManager` is pinned in the root manifest).

## Getting started

```sh
pnpm install
pnpm app:dev
```

The window opens with no engine and no weights, and says so. Start server unpacks the pinned build; the
model card offers to download the pack. The first answer streams from your machine, over loopback.

To build the app bundle: `pnpm app:build`.

## Checks

`./scripts/verify-precommit.sh` is the gate — fmt, clippy at `-D warnings`, the whole Rust test suite,
frontend format, lint, typecheck, store tests and build. It runs in CI on every PR and from
`.githooks/pre-commit`.

Alongside it, `verify-catalog.sh` re-checks the model contract (offline by default, against Hugging Face
when `HF_VERIFY_REMOTE=1`), `verify-tauri-ui.sh` and `verify-desktop-native.sh` boot the real window, and
`verify-studio-natively.sh` runs the whole set. `pnpm verify:all` is the long path.

Four of the checks exist because a claim that is not enforced rots within a commit:

- `check-i18n.mjs` — every key defined in both catalogs, and every literal lookup defined. The Rust side
  of the same promise is `verify-transcript-i18n`, which fails when a vocabulary the host can emit and
  the catalogs stop matching: a row's label (`StepKind`), a tool's one-line result (`SummaryKey`), a
  notice, a refusal's three sentences. Each also has to interpolate exactly the fields its code measures,
  because a row that renders a raw key — or a number that went out under a name the sentence never uses —
  is a bug in every language.
- `check-contrast.mjs` — every grey and every style ink at 4.5:1 on every surface each of the eight styles
  paints, in both modes.
- `check-shortcuts.mjs` — every binding in `lib/shortcuts.ts` answered by a handler, and every hint that
  names a key reading its label from the table.
- `check-settings-index.mjs` — the search index matching the rows each Settings page actually renders.

## Looking at the UI without clicking

The native window takes no injected clicks on a developer machine, and several states are one press deep.
The dev URL therefore accepts seeds that no shipped control ever sets:

| Seed | Opens |
| --- | --- |
| `?fixture=1` | a scripted run, so the transcript and inspector render with nothing to show yet |
| `?plan=1` | the run's plan panel, opened over the fixture's list |
| `?collapsed=<groups>` | the inspector's lower groups, which the status card otherwise pushes below the fold |
| `?settings=<page>`, `?q=<query>` | a Settings page, and the search field |
| `?review=1`, `?find=<text>`, `?scope=all`, `?outline=1` | the review panel, its find field, its scope and its fold |
| `?ask=<prompt>` | a real agent run against the local model, started from the URL |
| `?knowledge=<id>`, `?extensions=1` | a model's Knowledge Center page, and the Extensions inventory |

`pnpm app:dev:fixture`, `app:dev:settings` and `app:dev:chat` are the three pre-wired ones.

## Shipping

`pnpm app:build` produces an unsigned `.app` and a `.dmg` under `target/release/bundle/`. The disk image
puts the app and an Applications alias where the bundled background's arrow points; a second launch never
starts a second app, because the new process hands over and the open window comes forward — one process
owns the engine, the conversation and the saved key.

Two release steps are not wired yet, and this build makes no claim to have them:

- **Signing and notarisation.** The bundle is ad-hoc signed, so a copy downloaded from the internet is
  blocked by Gatekeeper until it is allowed in System Settings → Privacy & Security. A Developer ID
  Application certificate and notarisation credentials are not in this repository; they are supplied to
  Tauri through its `APPLE_*` environment at build time.
- **The updater.** It needs a published update endpoint and a signing key pair whose public half ships in
  the app. Neither exists here, so the app does not check for updates. A placeholder endpoint or key would
  look like a working update path while being none.

## Layout

```
crates/smollm-config    settings document, privacy and approval policy
crates/smollm-models    the catalog contract and its verification
crates/smollm-llama     engine launch plan, supervision, log reading
crates/smollm-tools     the tool registry, the sandbox, the two web calls
crates/smollm-core      the transcript model, the agent loop, approvals
apps/desktop            the Tauri v2 host and the React interface
docs/reference-ui       the screenshots the interface is matched against
docs/KNOWN_ISSUES.md    every model limit the code defends against, and where the defence is
```

## Third-party

- **llama.cpp and the PrismML fork of it** — the engine this app downloads and supervises; it is not
  bundled in this repository, and stays under its own license.
- **Model weights** from [`prism-ml` on Hugging Face](https://huggingface.co/prism-ml), downloaded by the
  user, sized and digest-pinned in `catalog/models.toml`.
- **Simple Icons (CC0)** — the path data behind three site marks in the inspector (Hugging Face, GitHub,
  Wikipedia), copied into `apps/desktop/src/components/icons.tsx`. The signs remain their owners'
  trademarks; only the icon geometry is shared.
- **KaTeX** fonts and stylesheet, bundled with the frontend build.

OpenFree is MIT licensed; see [`LICENSE`](LICENSE). The engine and the model weights keep their own
terms, listed above.
