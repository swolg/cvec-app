#!/usr/bin/env bash
set -euo pipefail

if [[ "${CVEC_LIVE_SMOKE_CONFIRM:-}" != "1" ]]; then
  echo "Refusing to send live repository context without explicit confirmation." >&2
  echo "See docs/ai-connections.md for the disposable repository example." >&2
  exit 2
fi

repository=${CVEC_LIVE_GROK_REPOSITORY:-}
if [[ -z "$repository" || ! -e "$repository/.git" ]]; then
  echo "Set CVEC_LIVE_GROK_REPOSITORY to the root of a disposable Git repository. See docs/ai-connections.md." >&2
  exit 2
fi

if [[ "$repository" != /* ]]; then
  repository="$PWD/$repository"
fi
export CVEC_LIVE_GROK_REPOSITORY="$repository"
export CVEC_RUN_LIVE_GROK_SMOKE=1

echo "Using the existing Grok CLI authentication. This opt-in smoke sends bounded repository context to xAI/Grok."
echo "Repository: $repository"

cargo test \
  --manifest-path src-tauri/Cargo.toml \
  --locked \
  live_grok_smoke_discovery_model_turn_usage_interrupt_and_resume \
  -- --ignored --nocapture --test-threads=1
