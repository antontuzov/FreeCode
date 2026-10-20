# Known issues, and what this app does about each one

Comments across `crates/`, `catalog/` and `engines/` cite `KNOWN_ISSUES.md`. This is the file they
mean, and it is an index rather than a copy: every entry names the upstream report, what breaks
without the workaround, and the code in this repository that carries the workaround. The upstream
statuses below are as of the date it was last checked, so re-read the source before deleting an entry
that has since been fixed in a release.

**The two upstream documents.** The model's own list is
<https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf/raw/main/KNOWN_ISSUES.md>
("Last checked: 2026-09-23"), and it is what a bare `KNOWN_ISSUES.md` citation refers to. Its issue
numbers are `PrismML-Eng/llama.cpp` for the runtime and `PrismML-Eng/Bonsai-demo` for the setup
scripts. `catalog/models.toml` names the same URL in its header, and `engines/lock.toml` records the
build this app ships.

**How to re-check whether an entry still applies.** Upstream's list describes releases older than
this app's pin, so "fixed in source but not in a release" is a claim about a different binary. Test
it against the pin instead of trusting the prose: the fork's compare endpoint answers the ancestry
question directly —
`GET https://api.github.com/repos/PrismML-Eng/llama.cpp/compare/<pin-commit>…<fix-commit>` returns
`status: "behind"` when the fix is already an ancestor of the pinned build. Re-run for the fixes the
pin is asserted to carry on 2026-10-20: #245 `0324c665`, #238 `3ae4f510`, #235 `ccbf5a75`, #252
`c53526a0`, #291 `46d25326`, #206 `01ae597e`, #196 `922be44a`, #317 `8b0c19c8`, and the tag
`prism-b10735-842b188` — all behind, so all present. The list was run again against
`73bfe6cc485554822f80c473a3c0c4f25e80f41b` when `engines/lock.toml` moved to `prism-b10787-73bfe6c`
the same day, and every one is still behind. **An entry in this file therefore cannot argue
for a from-source build on the grounds that the fix is unreleased**; if it needs one now, it is
because upstream's tracker still shows the item open, and that is what the wording says.

Every row was re-read that way on **2026-10-20**, together with the tracker states of the items the
catalog names. Still open there: #192, #199, #201, #213, #222, #223, #264 and #270 — so their rows
stay, and their wording says what upstream actually measured rather than what an older release did.
Two of them have no answer in the pinned build at all and are the only entries that point off it:
llama.cpp#199 (RTX 5090 / sm_120 prebuilt Linux CUDA builds, fixed only by building from source with
`-DCMAKE_CUDA_ARCHITECTURES=120a-real`) and llama.cpp#264 (RDNA2 under HIP, which upstream says has
no workaround on that backend — swap to the Vulkan build; note that upstream's own row still cites
Bonsai-demo#197, and the tracker redirects it). #241, which the model's list also calls open, is not:
it was closed by #271 in `prism-b10743`, before this pin.

## Replies that come back empty or cut off

**A small output cap ends the run mid-thought.** The reasoning counts against the limit, so a tight
`max_tokens` yields an empty or truncated reply — upstream calls this the most common report, and
notes it is often filed as "poor output in non-English languages".
_Enforced_: `GenerationBudget::default` is 16 384 tokens with a 32 768 window, and
`GenerationBudget::clamped` refuses to go under `MIN_MAX_TOKENS` (4 096) while keeping the window at
least 1 024 tokens above the reply budget — `crates/smollm-config/src/sampling.rs`.
_Not the card's exact numbers_: upstream pairs `-n 16384` with `-c 65536`; this app's default window
is 32 768, because the window is sized from the machine below.
_New in the 2026-10-20 pin, not yet used_: `73bfe6cc` adds `--reasoning-max-tokens-floor N`, which
raises a client `max_tokens` below `N` up to `N` server-side when thinking is on — the same job as
`MIN_MAX_TOKENS`, done where the launch is owned. The client-side clamp stays because it also covers
a shared server this app did not start; adopting the switch is filed as its own change rather than
smuggled into the bump.

**`reasoning_effort: "high"` is an HTTP 500.** The template knows `low`, `medium`, `xhigh`, `none`.
_Enforced_: the catalog declares `never_send = ["high"]` and `wire_effort` rejects it before a
request is built — `crates/smollm-llama/src/client.rs`. Test: `high_is_never_sent_and_low_goes_out_as_xhigh`.
_New in the 2026-10-20 pin, not yet used_: the same release adds `--reasoning-effort-allow` and
`--reasoning-effort-fallback`, so a server can substitute a safe level for a rejected one instead of
raising. `never_send` stays: rejecting on the client is what lets the control say what it will
actually send, and the switch only protects a server this app started.

**`low` does not shorten reasoning.** Upstream measured it at about the same cost as `xhigh`, so
offering it as "brief" would promise something untrue.
_Enforced_: `low` goes out as the catalog's `low_behaves_like` (`xhigh`) and the control is labelled
for what it sends — `ReasoningEffort::wire_value` in `crates/smollm-config/src/sampling.rs`. A
template with no thinking branch declares `low_behaves_like` empty, and no synonym is invented for
it.

**A thinking budget inside `chat_template_kwargs` is ignored silently.** This app sends no such
field; the budget is the reply cap above, and turning thinking off is `reasoning_effort: "none"` on
a server left at `--reasoning auto`.

**`--reasoning on` overrides a client's `reasoning_effort: "none"`.**
_Enforced_: the launcher emits neither `on` nor `off`, so `auto` (llama.cpp's own default) stays in
force — `crates/smollm-llama/src/launch.rs`. Test:
`no_reasoning_flag_is_emitted_because_auto_is_the_only_safe_default`.

## Tool calling and the agent loop

**Empty or non-JSON `arguments` are an HTTP 500/400** (llama.cpp#112), and this model does emit
them. _Enforced_: `ToolCall::arguments_wire` replays every call with a JSON object, `"{}"` at worst
— `crates/smollm-tools/src/registry.rs`. Test:
`an_unfinished_tool_call_is_replayed_as_an_empty_object`.

**Two system messages, or one that is not first, are an HTTP 500.**
_Enforced_: `normalize` in `crates/smollm-llama/src/client.rs` merges them into one leading message.

**Reasoning re-rendered into later turns misses the prompt cache every turn** (Bonsai-demo#183).
_Enforced_ from two sides: the client replays an assistant turn as its text plus its tool calls and
never as `reasoning_content`, and `--reasoning-preserve` — which the pinned build proposes on
startup — is kept out of the argument list for the same reason. Tests:
`a_replayed_assistant_turn_carries_no_reasoning`, and the ignored integration test
`crates/smollm-llama/tests/real_bonsai_server.rs`, whose
`a_second_turn_remembers_the_first_without_replaying_its_reasoning` asks a real second turn on a real
server.

**Tool calls sometimes come out malformed or looping** (repeated `//`), which upstream records as an
open model limitation. This app does not claim to fix it: the only defence is the `arguments_wire`
normalisation above, which keeps a malformed call from turning into an HTTP 500 on the replay.

## Loading the right binary for the right file

**`PQ2_0` and `PTQ1_0` are fork-only quant types.** Stock llama.cpp, Ollama and LM Studio reject
them; the `F16` reference *loads successfully in stock llama.cpp and generates nonsense*, because its
activation transform depends on metadata only the PrismML build applies.

**An ordinary model quantized to `PQ2_0` loads with no warning and emits garbage** (llama.cpp#242):
`llama-quantize` offers the type, but only a Prism export carries `prism.hadamard.*`.

_Enforced_: the load guard refuses a pack whose quant type is not the fork's, or whose GGUF lacks
every prefix the catalog entry requires, before any weight file reaches any binary —
`crates/smollm-llama/src/load_guard.rs`. Warnings never block a launch. They are the pack's
`[[model.packs.caveats]]` rows, and `caveat_matches` evaluates only the atoms `backend == <name>`,
`host_has_avx512`, and the OS names `windows` / `macos` / `linux`; a row that also names `gpu == …`
or an engine release is shown under "may or may not apply" rather than dropped, because failing open
beats quietly hiding a documented crash. The card splits the two in
`apps/desktop/src-tauri/src/status.rs`.

**ROCm/HIP aborts during load on consumer RDNA2 cards** (RX 6000 / gfx1030), whatever the packing
(llama.cpp#264 — upstream's row still cites Bonsai-demo#197, which the tracker redirects — open as of
2026-10-20). _Enforced_: only as a caveat, on every runtime pack a HIP host can be routed to, saying
there is nothing to apply on HIP and the Vulkan build is the way. **What this app does not do**: it
does not override the probed backend, so a machine whose probe chooses HIP still has to swap engine
builds itself.

**RTX 5090 (sm_120) prebuilt Linux CUDA builds decode erratically** (llama.cpp#199, open), reported
on both ternary bands, and this is the one entry on upstream's list whose recorded fix really is a
source build: `-DCMAKE_CUDA_ARCHITECTURES=120a-real`. Nothing in the pin addresses it. _Enforced_:
as a caveat on the 27B's two runtime packs, plus `engines/lock.toml`'s `[from_source]` table.

**SYCL has no binary for this app to run.** Upstream used to report "unsupport data type" for both
ternary types there; llama.cpp#235 merged ternary SYCL loading on 2026-09-24 and llama.cpp#356 added
the TernSYCL XMX int2×int8 path, decode attention and fusions, both inside the pin — and the release
ships `win-sycl-x64` / `ubuntu-sycl-*` archives, with docs/backend/SYCL-bonsai.md publishing a
measured Arc B390 run of PTQ1_0 on SYCL. So this is no longer a gap in the kernels. It is still declined, on three facts that are about
this app rather than about Intel hardware: `engines/lock.toml` pins no SYCL asset and the archives
need oneAPI runtime libraries the app does not ship; SYCL-RELEASE.md says flatly that "compilation
does not validate model correctness or performance on an Intel GPU", BACKEND-SUPPORT.md keeps the
ternary cells at ✅* (partial / needs validation) on a source audit baseline older than the release,
and SYCL-bonsai.md's own measurement ends with "other Intel GPUs have not been measured with these
defaults; please report numbers"; and even the demo's own scripts never select it.
_Enforced_: the lock marks SYCL unsupported with that reason, and `EngineBinary::resolve_for` returns
`UnsupportedBackend` rather than launching it — `crates/smollm-llama/src/engine.rs`. Test:
`sycl_is_refused_with_the_reason_from_the_lock_file`.

**CUDA 13.3 builds crash on load; on Windows the server prints its banner and exits silently**
(llama.cpp#222). _Enforced_: the lock marks one asset per backend `recommended`, and auto-resolution
picks it — Windows/CUDA resolves to 12.4. Test:
`auto_resolution_prefers_the_recommended_asset_for_a_backend`.

**`PQ2_0` segfaults while loading on AVX-512 CPUs** (Zen 4/5, Strix Halo) — **on releases before
llama.cpp#245**, merged 2026-09-23 and an ancestor of this app's pin, so the pinned build carries the
fix. The warning `score_pack` emits on such a host is therefore about what has never been *measured
here*, not about a crash the user is about to hit. This app ships `prism-b10787-73bfe6c`, newer than
the release upstream names as the workaround, and the pin is asserted to be `>= prism-b10709` because
the launch-day `prism-b10687` shipped no binaries at all. **What this app cannot check**: whether this
machine is an affected CPU, and whether the fixed kernel path actually decodes on GPU there. The
pack's `condition` strings are prose for a human to read, not rules the host is matched against —
`apps/desktop/src-tauri/src/status.rs` lists any caveat it cannot evaluate under `unchecked_warnings`
("may or may not apply"), while the decision that avoids the risk is made in code by `score_pack`.

## Performance and context

**A single conversation keeps re-processing its prompt** when the server splits its cache across
slots (Bonsai-demo#183). _Enforced_: a private launch is started with `-np 1 --cache-ram 24576`; a
shared server is not, because it is somebody else's slot count. Test:
`one_dedicated_slot_with_a_prompt_cache_is_the_documented_single_user_launch`.

**`q5_0` KV cache is several times slower at every depth** (llama.cpp#191).
_Enforced_: the catalog names the types worth using, and `LaunchPlan::assemble` rejects anything else
with `KvCacheRejected` — `crates/smollm-llama/src/launch.rs`.

**The default 32K context does not fit on 12 GB GPUs**, because the demo scripts size the context
from system RAM (Bonsai-demo#193). _Partly enforced_: `context_from_ram` tiers the window by machine
(8 GiB → 8 192 … 128 GiB → 131 072) and never emits `-c 0`, and the pack's caveats carry the GPU
half of the decision into the Model Manager UI. **What this app does not do**: size the window from
VRAM. A 12 GB GPU in a 16 GB machine still gets a 16 384 window from the RAM tier, so the offload
plan and the caveats — not the tier — are what keep it loadable.

**`PQ2_0` on Vulkan used to silently run on the CPU** at under 2 tok/s in the binary releases, while
the log still claimed every layer was offloaded (llama.cpp#238, merged 2026-09-21). Upstream's note
says "not in a release at the time of writing"; `#238` is an ancestor of this app's pin, so on the
build this app installs the fallback this row describes is not the risk — what is left is
BACKEND-SUPPORT.md marking that cell partial (kernel selection depends on the device and driver,
no direct coopmat2 decoder, and the row's audit baseline is `prism-b10743-adfffbe` rather than the
release it describes), plus #192 and #270,
which stay open on Intel parts. _Enforced_ before the packing is ever chosen: `score_pack` penalises
`PQ2_0` on a Vulkan host so `PTQ1_0` is selected, and the warning says plainly that this is this app's
unmeasured caution rather than a known failure
(`crates/smollm-models/src/select.rs`, test `vulkan_never_gets_pq2`). If a ternary run on a Vulkan
part is ever measured here, the rule and the wording change together.

## Vision

**Grounding tasks do badly on small text in a picture**; the card's fix is `--image-min-tokens 1024`.
This app deliberately sends the opposite bound — `--image-max-tokens 1024` (`IMAGE_TOKEN_CAP`) beside
the `--mmproj` — on Metal, Vulkan and CPU, because that is what the demo's own `common.sh` does for
latency, and leaves images uncapped on CUDA and ROCm. Its problem is the context budget: a
projector's expansion has to be charged to the window before the prompt is sent, and
`chat.rs::picture_tokens` charges the same 1024 per picture so the pre-flight warning and the server
agree — where the cap is off, that is deliberately the cheap reading of the cost, since a warning that
arrives late beats one that never arrives. **Not enforced**: no floor is set on any backend, so a
small, dense image gets fewer tokens than upstream would recommend. Two of the three catalog entries
carry a vision projector — the 27B and the 4B, the latter marked `image-text-to-text` — while the 1.7B
that this machine runs by default is text-only, so the trade is only visible after the Model Manager
has been asked for pictures.
