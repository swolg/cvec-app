#!/bin/sh
set -eu
cvec_script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
node "$cvec_script_dir/local-signing.mjs" sign "$1"
exec "$@"
