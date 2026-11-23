# Getting started

Follow the [macOS build instructions](../README.md#build-and-run-on-macos) to build and run CVEC locally.

## First run

1. **Choose a repository folder:** Open **Repositories → Choose folder** (or **Settings → General → Local repositories**) and select a local Git repository or parent folder (defaults to `~/code`).
2. **Run a scan:** Make sure `syft` and `grype` are installed, then choose **Scan all**. The first run downloads the Grype vulnerability database and can take a moment.
3. **Review findings:** Open **Review** to inspect affected packages, remediation evidence, and mark findings as reviewed or snoozed. Closing the window keeps CVEC running in the menubar.
4. **Public advisories (optional):** Open **Intelligence**, pick watched topics, and choose **Fetch advisories** to compare public advisories against packages found in your local scans.
5. **Private GitHub repos (optional):** Follow the [GitHub connection guide](github-connection.md) to connect your own GitHub App, or clone repositories locally under your watched folder.
6. **AI chat and Fix workspaces (optional):** Configure Codex, Grok, or OpenCode in [AI connections](ai-connections.md). Try [Fix workspaces](fix-workspaces.md) on a clean, disposable Git repository first and review diffs and rescan results before applying changes.

## Known limitations

- **macOS first:** Targets macOS 12+; native testing has focused on Apple Silicon.
- **Updating:** Quit CVEC from the menubar, [back up your app data](local-data.md#back-up-your-app-data), pull the latest `main`, and rebuild.
- **Scanner coverage:** Results depend on your project's manifests/lockfiles, Syft/Grype versions, and advisory DB freshness.
- **Transitive Cargo exceptions:** A small set of Linux-only or unmaintained transitive Cargo notices have documented exceptions; see [dependency assurance](dependency-assurance.md).
- **AI runtimes:** Analysis (read-only) and Fix workspaces have different requirements; see [runtime compatibility](fix-workspaces.md#runtime-compatibility). Live Grok Fix behavior is currently unverified beyond automated adapter tests.

## Feedback

Report reproducible bugs via the repository's issue form with your commit SHA, macOS/tool versions, and reproduction steps. Redact private repo names, source code, and credentials before attaching logs or screenshots.

For suspected security vulnerabilities, use the [private security reporting channel](../SECURITY.md).
