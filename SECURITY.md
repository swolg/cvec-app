# Security Policy

## Reporting a vulnerability

Please do **not** report suspected security vulnerabilities in public issues.

Use **Security → Advisories → Report a vulnerability** in this repository to submit a confidential report via GitHub Private Vulnerability Reporting.

Please include:

- The commit SHA of your Git checkout, macOS version, and tool versions
- Reproduction steps using a disposable example
- Expected security impact and any suggested mitigation

Remove real credentials, private source code, and unrelated personal data—never attach a production database or unredacted scan/chat logs.

## Supported versions

Security fixes target the latest `main` branch.

## Scope

In scope for CVEC:

- The desktop app, local SQLite data handling, and macOS Keychain credential storage
- Dependency scanning orchestration and GitHub repository materialization
- Read-only Analysis boundaries, context redaction, and isolated Fix workspaces (tool permissions, worktree isolation, sandboxed checks, cancellation)

Out of scope: upstream advisory feeds, `syft`, `grype`, external AI CLIs/providers, or GitHub API behavior (report those upstream unless CVEC handles their output unsafely). There is no hosted CVEC backend or API.

## Local-first security model

- **Local storage:** Repository metadata, scan artifacts, findings, settings, and cached chat timelines stay in local SQLite.
- **Direct external calls:** Requests go directly from the desktop app to advisory feeds, scanner DB updates, optional GitHub APIs, or—after explicit consent—the selected AI runtime (`Codex`, `Grok`, or `OpenCode`).
- **AI disclosures:** Analysis sends the prompt and bounded, redacted repository context. Fix workspaces can additionally send agent-selected worktree source, visible history, and check output under a separate disclosure and can edit the dedicated local worktree through CVEC's restricted tools. Prompts are stored as submitted, so never paste secrets into Chat. A local CLI does not imply local model inference.
- **Credentials:** GitHub access tokens and optional OpenCode loopback passwords are stored in the macOS Keychain. External AI provider credentials remain owned by their respective CLI/runtime and are never stored in SQLite.

See [AI connections](docs/ai-connections.md), [Fix workspaces](docs/fix-workspaces.md), and [Optional GitHub access](docs/github-connection.md) for details.
