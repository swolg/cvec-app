#!/usr/bin/env bash
set -euo pipefail

if [[ "${CVEC_OPENCODE_SMOKE_ACK:-}" != "I_ACCEPT_PROVIDER_DATA_SEND" ]]; then
  echo "Refusing to contact an AI provider. Set CVEC_OPENCODE_SMOKE_ACK=I_ACCEPT_PROVIDER_DATA_SEND after reviewing the selected OpenCode provider/model boundary." >&2
  exit 2
fi

if [[ -z "${CVEC_OPENCODE_SMOKE_MODEL:-}" ]]; then
  echo "Set CVEC_OPENCODE_SMOKE_MODEL to an explicit provider/model id such as ollama/qwen3." >&2
  exit 2
fi

smoke_dir="$(mktemp -d)"
trap 'rm -rf -- "$smoke_dir"' EXIT

opencode run \
  --format json \
  --model "${CVEC_OPENCODE_SMOKE_MODEL}" \
  --dir "$smoke_dir" \
  'Read-only cvec provider smoke test. Reply with exactly: connection ready. Do not use tools.'
