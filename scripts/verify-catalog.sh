#!/usr/bin/env bash
# verify-hf-gguf-catalog-entry: every catalog entry that is ternary (PTQ1_0 / PQ2_0) must be
# flagged needs_fork, must resolve to a fork engine asset, and must never be servable by a stock
# llama.cpp. With HF_VERIFY_REMOTE=1 it also asks Hugging Face for the real object sizes and digests.
source "$(dirname "$0")/_env.sh"

ARGS=(.)
if [[ "${HF_VERIFY_REMOTE:-0}" == "1" ]]; then
  step "checking the catalog against Hugging Face"
  ARGS+=(--remote)
else
  step "checking the catalog against the pinned contract (set HF_VERIFY_REMOTE=1 to hit Hugging Face)"
fi

cargo run --locked -p freecode-models --bin verify-hf-gguf-catalog-entry -- "${ARGS[@]}"
