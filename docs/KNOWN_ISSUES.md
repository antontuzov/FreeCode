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

## Replies that come back empty or cut off

**A small output cap ends the run mid-thought.** The reasoning counts against the limit, so a tight
`max_tokens` yields an empty or truncated reply — upstream calls this the most common report, and
notes it is often filed as "poor output in non-English languages".
_Enforced_: `GenerationBudget::default` is 16 384 tokens with a 32 768 window, and
`GenerationBudget::clamped` refuses to go under `MIN_MAX_TOKENS` (4 096) while keeping the window at
least 1 024 tokens above the reply budget — `crates/smollm-config/src/sampling.rs`.
_Not the card's exact numbers_: upstream pairs `-n 16384` with `-c 65536`; this app's default window
is 32 768, because the window is sized from the machine below.

**`reasoning_effort: "high"` is an HTTP 500.** The template knows `low`, `medium`, `xhigh`, `none`.
_Enforced_: the catalog declares `never_send = ["high"]` and `wire_effort` rejects it before a
request is built — `crates/smollm-llama/src/client.rs`. Test: `high_is_never_sent_and_low_goes_out_as_xhigh`.

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
`crates/smollm-llama/src/load_guard.rs`. Warnings never block a launch; they are the pack's
`[[model.packs.caveats]]` rows matched against this host, which is what the status card lists.

**The SYCL build cannot load either ternary type at all** (llama.cpp#235, "unsupport data type").
_Enforced_: `engines/lock.toml` marks SYCL unsupported with the reason copied verbatim, and
`EngineBinary::resolve_for` returns `UnsupportedBackend` rather than launching it —
`crates/smollm-llama/src/engine.rs`. Test: `sycl_is_refused_with_the_reason_from_the_lock_file`.

**CUDA 13.3 builds crash on load; on Windows the server prints its banner and exits silently**
(llama.cpp#222). _Enforced_: the lock marks one asset per backend `recommended`, and auto-resolution
picks it — Windows/CUDA resolves to 12.4. Test:
`auto_resolution_prefers_the_recommended_asset_for_a_backend`.

**`PQ2_0` segfaults while loading on AVX-512 CPUs** (Zen 4/5, Strix Halo; fixed in source by
llama.cpp#245, merged 2026-09-23). This app ships `prism-b10772-e8fd209`, newer than the release
upstream names as the workaround, and the pin is asserted to be `>= prism-b10709` because the
launch-day `prism-b10687` shipped no binaries at all. **What this app cannot check**: whether this
machine is an affected CPU — the caveat is shown as a risk on the host that matches it, not as a
claim that it was reproduced.

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

**`PQ2_0` on Vulkan silently runs on the CPU at under 2 tok/s in the binary release** (fixed in
source by llama.cpp#238, merged 2026-09-21, not in a release at the time of writing). Decided before
the packing is ever chosen: `score_pack` penalises `PQ2_0` on a Vulkan host and warns that the GPU log
is misleading — watch the `CPU model buffer size` — so `PTQ1_0` is selected instead
(`crates/smollm-models/src/select.rs`).

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
