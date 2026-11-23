# Dependency advisory exceptions

CVEC targets macOS. The active Cargo/OSV audit exceptions below are upstream Linux-only or unmaintained transitive dependencies; they are configured in [dependency-policy.json](../../security/dependency-policy.json) and [cargo-audit.sh](../../scripts/cargo-audit.sh).

| Advisory | Locked package | Reason and removal condition |
| --- | --- | --- |
| [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) (`GHSA-wrw7-89jp-8q8g`) | `glib 0.18.5` | Unsound string-variant iteration in the Linux Tauri/Wry/GTK3 stack (absent from the macOS dependency tree and unused by CVEC). Remove once upstream migrates to `glib 0.20+`. |
| [RUSTSEC-2024-0370](https://rustsec.org/advisories/RUSTSEC-2024-0370.html) | `proc-macro-error 1.0.4` | Unmaintained build-time crate pulled in via Linux GTK3/glib macros (absent on macOS). Remove when upstream macros migrate. |
| [RUSTSEC-2026-0370](https://rustsec.org/advisories/RUSTSEC-2026-0370.html) | `zvariant 5.12.0` | Unmaintained notice via Linux `zbus 5` notification stack. Remove once upstream adopts `zbus 6`. |
| [RUSTSEC-2026-0371](https://rustsec.org/advisories/RUSTSEC-2026-0371.html) | `zbus_names 4.3.2` | Same Linux `zbus 5` notification stack. Remove after upstream `zbus 6` migration. |
| [RUSTSEC-2026-0372](https://rustsec.org/advisories/RUSTSEC-2026-0372.html) | `zvariant_derive 5.12.0` | Same Linux `zbus 5` notification stack. Remove after upstream `zbus 6` migration. |
| [RUSTSEC-2026-0543](https://rustsec.org/advisories/RUSTSEC-2026-0543.html) | `chrono 0.4.45` | Unmaintained notice. CVEC uses `time`; `chrono` remains only in optional `serde_with` lockfile metadata with no enabled dependency path. Remove once it drops from `Cargo.lock`. |

These exceptions do not claim that an affected crate is sound or that Linux is supported. Re-evaluate and prune entries whenever `Cargo.lock` or upstream dependencies update.
