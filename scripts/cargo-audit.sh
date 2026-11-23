#!/usr/bin/env bash
set -euo pipefail

# The first five advisories are transitive through the current
# Tauri/Wry/GTK3 and tauri-utils stacks. The 2026-0370/0371/0372
# entries are unmaintained notices (not vulnerability reports); zbus 5 still
# depends on the standalone crates that were merged into zbus 6. Remove entries
# as upstream releases stop pulling the affected crates into Cargo.lock.
# RUSTSEC-2026-0543 remains in the lockfile through serde_with's optional
# dependencies. The enabled targets do not build chrono.
exec cargo audit \
  --file src-tauri/Cargo.lock \
  --ignore RUSTSEC-2024-0370 \
  --ignore RUSTSEC-2024-0429 \
  --ignore RUSTSEC-2026-0370 \
  --ignore RUSTSEC-2026-0371 \
  --ignore RUSTSEC-2026-0372 \
  --ignore RUSTSEC-2026-0543 \
  "$@"
