# Contributing

Thanks for helping improve CVEC!

## Setup

Follow the [macOS build instructions](README.md#build-and-run-on-macos) to install Node, Xcode Command Line Tools, Rust, `syft`, `grype`, and project dependencies (`npm ci`).

Set up the local macOS development signing identity once before launching the app:

```sh
npm run signing:setup
npm run signing:doctor
npm run tauri -- dev
```

See [Local development and Keychain](docs/local-development-keychain.md) for Keychain prompt details and signing overrides.

## Checks

Before opening a pull request, run the frontend and Rust checks:

```sh
npm run check
npm audit --omit=dev
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo install cargo-audit --version 0.22.2 --locked
./scripts/cargo-audit.sh --deny warnings
```

## Project guidelines

- **Stay local-first:** CVEC has no backend, account system, or cloud sync. Any optional network feature must be a documented direct desktop-to-third-party workflow. Keep CVEC-managed credentials in the OS Keychain, and leave external AI runtime credentials owned by their respective CLIs.
- **Keep PRs focused:** Include Rust or frontend tests for behavior changes, document user-visible workflows in `README.md` or `docs/`, and note any SQLite migration or data-compatibility impact.
- **Use synthetic fixtures:** Never include real credentials, private repositories, or personal data in tests, logs, or screenshots.
- **Dependencies:** After changing dependencies, run `npm run licenses:generate` to update `security/licenses/inventory.json`, and do not expand advisory exceptions to silence new findings.
- **Workflow safety:** Pull request workflows run read-only without signing or provider secrets—do not introduce privileged fork execution or secret requirements.

Suspected security vulnerabilities should be reported privately via [SECURITY.md](SECURITY.md). Please keep discussions constructive and respectful.
