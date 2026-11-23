# Local data and SQLite

On macOS, CVEC stores all application state locally:

- **App data:** `~/Library/Application Support/dev.cvec.app`
  - `cvec.db` — repositories, scan history, findings, review decisions, settings, chat cache, and fix workspace records
  - `fix-workspaces/` — isolated Git worktrees created by Fix mode
- **Cache:** Tauri app cache directory (`grype/db` for the Grype vulnerability DB, `inventory` for cached Syft SBOMs)
- **Credentials:** macOS Keychain (GitHub user access tokens and optional OpenCode loopback passwords)

## Back up your app data

Quit CVEC from the menubar first (closing the window leaves it running), then copy the entire app data directory so `cvec.db` and any SQLite WAL files stay together:

```sh
(
  set -eu
  data_dir="$HOME/Library/Application Support/dev.cvec.app"
  test -f "$data_dir/cvec.db"
  backup_dir=$(mktemp -d "$HOME/cvec-backup.XXXXXXXX")
  cp -pR "$data_dir" "$backup_dir/"
  printf 'Backup saved to %s\n' "$backup_dir"
)
```

Keep backups private—they contain your local scan, chat, and workspace history.

## Inspect a read-only copy of `cvec.db`

With CVEC quit, copy the database into a temporary directory and open it read-only:

```sh
(
  set -eu
  db="$HOME/Library/Application Support/dev.cvec.app/cvec.db"
  inspection_dir=$(mktemp -d)
  trap 'rm -rf -- "$inspection_dir"' EXIT
  cp -p "$db" "$inspection_dir/cvec.db"
  if [ -f "${db}-wal" ]; then
    cp -p "${db}-wal" "$inspection_dir/cvec.db-wal"
  fi
  sqlite3 -init /dev/null -bail "$inspection_dir/cvec.db" 'PRAGMA journal_mode = DELETE;' >/dev/null
  sqlite3 -init /dev/null -readonly "$inspection_dir/cvec.db"
)
```

Inside `sqlite3`, run `.tables` or `.schema <table_name>` to explore tables such as `repos`, `scan_runs`, `vulnerabilities`, `finding_reviews`, `companion_threads`, and `external_advisories`. Type `.quit` when finished.
