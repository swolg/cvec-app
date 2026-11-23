#!/bin/sh
set -eu
cvec_script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "${1:-}" = run ]; then
  cvec_runner=$(node "$cvec_script_dir/local-signing.mjs" runner-path)
  exec cargo --config "target.aarch64-apple-darwin.runner=$cvec_runner" --config "target.x86_64-apple-darwin.runner=$cvec_runner" "$@"
fi
exec cargo "$@"
