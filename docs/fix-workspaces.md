# Provider fix workspaces

Open **Chat → Fix** on a local repository to run an isolated remediation session. Each fix workspace has its own Git branch, worktree, conversation, diff, check results, and rescan evidence—leaving your main checkout untouched.

## Workflow

1. From a local finding with a known fix target, choose **Fix in Chat** (or open **Chat → Fix** directly).
2. Confirm **Create branch and worktree**. CVEC creates a branch from your clean local `HEAD` and stores the worktree in app data.
3. Select a compatible connection/model and submit the fix task after reviewing the source-sharing disclosure. You can click **Stop** at any time without losing generated files or results.
4. Inspect **Diff**, run supported **Checks** (`npm` / `cargo` scripts), and run **Rescan worktree** to verify with Syft and Grype.
5. Review the changes yourself before applying or cherry-picking them into your checkout. CVEC never commits, pushes, merges, or opens PRs automatically.

## Runtime compatibility

| Runtime | Fix integration | Requirements |
| --- | --- | --- |
| **Codex** | Built-in connection via app-server dynamic tools | Codex CLI `>= 0.166.0`; native shell/integrations disabled and verified before start |
| **Grok** | ACP session + private loopback MCP bridge | Blocking `pre_tool_use` + generated deny rules; no inherited hooks/plugins/MCP servers (live model behavior currently unverified) |
| **OpenCode** | Private managed V2 server + loopback MCP bridge | OpenCode V2+ with managed local mode and verified deny-all native policy |

An existing external OpenCode server cannot be used for Fix because CVEC cannot enforce tool isolation there.

## Security and sandbox boundaries

- **Restricted tools:** Providers can only use CVEC-owned worktree tools (bounded file read/edit, diff, supported dependency prep/checks, and rescan). Arbitrary shell commands, symlink/path traversal, binary/oversized files (`> 1 MiB`), Git metadata, and detected secrets are rejected.
- **macOS sandbox:** Check commands (`npm` `test:run`/`test`/`check`/`build`/`lint`/`typecheck`, Cargo `test`/`check`/`clippy`) run inside the macOS sandbox with network disabled for tests and private cache/temp directories. Dependency preparation actions (`npm` with lifecycle scripts disabled, Cargo fetch/update) are labeled separately.
- **Source sharing:** After confirmation, the fix task, finding evidence, agent-selected worktree files, recent visible messages, and check output are sent to the chosen model provider. A local CLI does not imply local inference.
- **Rescan evidence:** **Rescan worktree** runs Syft/Grype on the isolated worktree and compares against the baseline scan when available. It reports **not detected** rather than claiming exploitability or compatibility, and never mutates the original repository's Review state.
- **Worktree retention:** Up to 20 active fix workspaces are retained per repository. Idle workspaces can be archived (preserving their branch, files, messages, and results) and restored later.
