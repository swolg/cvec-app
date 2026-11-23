# AI connections and repository chat

CVEC Chat is a repository-scoped, read-only security assistant. Each conversation belongs to one repository, and you can switch models or connections mid-conversation while CVEC keeps the visible timeline cached locally in SQLite.

Separate [Fix workspaces](fix-workspaces.md) reuse these connections for explicit code-edit sessions inside an isolated Git worktree. Ordinary Analysis always stays read-only.

## Built-in connections

Open **Settings → AI Providers** to see automatically detected runtimes, discovered models, health status, and privacy boundaries. Provider credentials stay with each CLI/runtime—CVEC does not store vendor API keys in SQLite. (Do not paste secrets into prompts, as prompt text is saved as visible chat history.)

### Codex

Install the Codex CLI and sign in:

```sh
npm install -g @openai/codex
# or: brew install --cask codex
codex login
```

CVEC starts `codex app-server --stdio` lazily when you use Codex in Chat and runs Analysis in a read-only sandbox directory. ChatGPT browser login and device-code login can also be triggered from Chat when signed out. For a custom binary path, set `CVEC_CODEX_PATH` or configure an additional connection in **Settings → AI Providers**.

### Grok

Install the Grok CLI and authenticate with `grok login` (or inherit `XAI_API_KEY` in the environment). CVEC connects via `grok agent stdio` (ACP) with filesystem and terminal tools disabled during Analysis.

### OpenCode and local models

Install OpenCode (`>= 1.0.0`) and configure your model providers in OpenCode itself (Ollama, LM Studio, llama.cpp, or BYOK cloud providers), then refresh OpenCode in **Settings → AI Providers**.

- **Managed local mode** runs OpenCode in a CVEC-managed sandbox directory with a deny-all Analysis policy.
- **Existing loopback server** (OpenCode V1 only) accepts credential-free loopback URLs (`http://127.0.0.1:<port>`, `http://[::1]:<port>`, `http://localhost:<port>`). Optional server passwords are stored in the macOS Keychain.

## Privacy boundaries and consent

Each connection is labeled with a boundary class:

- `verified local` — runtime metadata confirms a loopback-only model endpoint.
- `external` — inference is sent to a remote provider.
- `unknown` — CVEC cannot prove either condition.

Opening Chat or inspecting context sends nothing. Before the first send on a connection (or whenever the provider/boundary changes), CVEC shows a disclosure naming the destination and bounded repository context categories. In Analysis mode, CVEC denies provider tools so the model cannot edit files, run commands, or change Review decisions.

## Troubleshooting

- **Runtime unavailable:** Verify `codex`, `grok`, or `opencode` is installed and press **Refresh** in **Settings → AI Providers**.
- **Signed out / setup required:** Run `codex login` or `grok login` (or sign in from Chat for Codex), then refresh.
- **No models listed in OpenCode:** Configure a provider in OpenCode and ensure your local model server (e.g. Ollama / LM Studio) is running, then refresh.
- **History unavailable:** If a provider-owned session cannot be resumed, CVEC keeps the locally cached messages readable and starts a fresh provider segment on your next message.
- **Offline / provider outage:** Local scanning, Review, and repository inventory continue to work independently.

### Optional live provider smoke tests

To test a live provider against a disposable Git repository, run the command for the provider you want to test:

```sh
(
  set -eu
  smoke_repo=$(mktemp -d)
  trap 'rm -rf -- "$smoke_repo"' EXIT
  git init --quiet --template= "$smoke_repo"
  printf '%s\n' '# CVEC live smoke fixture' > "$smoke_repo/README.md"

  # Run whichever provider CLI you have installed:
  CVEC_LIVE_SMOKE_CONFIRM=1 CVEC_LIVE_CODEX_REPOSITORY="$smoke_repo" scripts/companion-live-smoke.sh
  # CVEC_LIVE_SMOKE_CONFIRM=1 CVEC_LIVE_GROK_REPOSITORY="$smoke_repo" scripts/grok-live-smoke.sh
)

CVEC_OPENCODE_SMOKE_ACK=I_ACCEPT_PROVIDER_DATA_SEND CVEC_OPENCODE_SMOKE_MODEL=ollama/qwen3 scripts/opencode-live-smoke.sh
```
