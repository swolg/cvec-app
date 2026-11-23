import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProviderConnectionSnapshot } from "@/lib/types";
import { AiConnectionsSettings } from "./AiConnectionsSettings";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  validate: vi.fn(),
  save: vi.fn(),
  refresh: vi.fn(),
  enable: vi.fn(),
  reset: vi.fn(),
  remove: vi.fn(),
  consent: vi.fn(),
  setConsent: vi.fn(),
}));

vi.mock("@/lib/providerConnectionsApi", () => ({
  listProviderConnections: mocks.list,
  validateProviderConnection: mocks.validate,
  saveProviderConnection: mocks.save,
  refreshProviderConnection: mocks.refresh,
  setProviderConnectionEnabled: mocks.enable,
  resetProviderConnection: mocks.reset,
  removeProviderConnection: mocks.remove,
  providerConnectionConsent: mocks.consent,
  setProviderConnectionConsent: mocks.setConsent,
}));

function connection(): ProviderConnectionSnapshot {
  return {
    schemaVersion: 1,
    connection: { schemaVersion: 1, id: "opencode_local", driver: "open_code", displayName: "Local models", accentColor: null, enabled: true, builtIn: false, config: { kind: "open_code", mode: "managed_local", binaryPath: null, serverUrl: null }, hasServerPassword: false, createdAt: "now", updatedAt: "now" },
    health: "ready",
    installedVersion: "1.18.18",
    authKind: "none",
    accountLabel: null,
    capabilities: { createThread: true, resumeThread: true, interrupt: true, orderedStreaming: true, readOnlyPolicy: true, accountStatus: false, rateLimits: false, tokenUsage: true, reasoningEffort: false, attachments: true },
    upstreamProviders: [{ id: "ollama", displayName: "Ollama", connected: true, inferenceBoundary: "verified_local" }],
    models: [{ connectionId: "opencode_local", driver: "open_code", upstreamProviderId: "ollama", inferenceBoundary: "verified_local", id: "ollama/qwen3", displayName: "Qwen 3", description: null, isDefault: true, hidden: false, reasoningEfforts: [], inputModalities: ["text"], contextWindow: 32768 }],
    checkedAt: "2026-08-20T10:00:00Z",
    message: "OpenCode is ready.",
    error: null,
  };
}

function codexConnection(): ProviderConnectionSnapshot {
  const base = connection();
  return {
    ...base,
    connection: {
      ...base.connection,
      id: "codex_default",
      driver: "codex",
      displayName: "Codex",
      builtIn: true,
      config: { kind: "codex", binaryPath: null, homePath: null, profile: null, launchArgs: null },
    },
    authKind: "chatgpt",
    upstreamProviders: [{ id: "openai", displayName: "OpenAI", connected: true, inferenceBoundary: "external" }],
    models: [{ ...base.models[0], connectionId: "codex_default", driver: "codex", upstreamProviderId: "openai", inferenceBoundary: "external", id: "gpt-5", displayName: "GPT-5" }],
  };
}

function grokConnection(): ProviderConnectionSnapshot {
  const base = connection();
  return {
    ...base,
    connection: {
      ...base.connection,
      id: "grok_default",
      driver: "grok",
      displayName: "Grok",
      builtIn: true,
      config: { kind: "grok", binaryPath: null, customModels: [] },
    },
    health: "setup_required",
    installedVersion: "1.0.5",
    upstreamProviders: [{ id: "xai", displayName: "xAI", connected: false, inferenceBoundary: "external" }],
    models: [{ ...base.models[0], connectionId: "grok_default", driver: "grok", upstreamProviderId: "xai", inferenceBoundary: "external", id: "grok-build", displayName: "Grok Build" }],
    message: "Sign in with the Grok CLI.",
  };
}

function defaultOpenCodeConnection(): ProviderConnectionSnapshot {
  const base = connection();
  return {
    ...base,
    connection: {
      ...base.connection,
      id: "opencode_default",
      displayName: "OpenCode",
      builtIn: true,
    },
    health: "unavailable",
    installedVersion: null,
    upstreamProviders: [],
    models: [],
    message: null,
    error: { code: "not_installed", message: "OpenCode is not installed or is not on PATH.", retryable: true },
  };
}

describe("AiConnectionsSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.list.mockResolvedValue([connection()]);
    mocks.validate.mockResolvedValue(connection());
    mocks.save.mockResolvedValue(connection());
    mocks.enable.mockResolvedValue({ ...connection(), connection: { ...connection().connection, enabled: false } });
    mocks.refresh.mockResolvedValue(connection());
    mocks.consent.mockResolvedValue({ provider: "test", disclosureVersion: 1, acceptedAt: null, revokedAt: null, valid: false });
    mocks.setConsent.mockResolvedValue({ provider: "test", disclosureVersion: 1, acceptedAt: null, revokedAt: null, valid: false });
  });

  it("renders a compact provider summary with expandable details and an enabled switch", async () => {
    render(<AiConnectionsSettings />);
    expect(await screen.findByRole("article", { name: "Local models AI connection" })).toBeInTheDocument();
    expect(screen.getByText("Available · 1 upstream provider · 1 model · verified local")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Disable Local models" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Show Local models details" }));
    expect(screen.getByText("Runtime")).toBeInTheDocument();
    expect(screen.getByText("OpenCode")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configure" })).toBeInTheDocument();
  });

  it("names each wizard step and distinguishes valid configuration from missing models", async () => {
    mocks.validate.mockResolvedValueOnce({ ...connection(), models: [], message: "Configuration is structurally valid." });
    render(<AiConnectionsSettings />);
    await screen.findByRole("article", { name: "Local models AI connection" });
    fireEvent.click(screen.getByRole("button", { name: "Add another" }));
    const choice = await screen.findByRole("dialog", { name: "Choose a runtime" });
    fireEvent.click(within(choice).getByRole("button", { name: "Continue" }));
    const configure = await screen.findByRole("dialog", { name: "Configure connection" });
    fireEvent.click(within(configure).getByRole("button", { name: "Validate" }));
    const review = await screen.findByRole("dialog", { name: "Review and save" });
    expect(within(review).getByText("No usable model")).toBeVisible();
    expect(within(review).getByText(/Configure a model in OpenCode/)).toBeVisible();
    expect(within(review).getByRole("button", { name: "Save setup" })).toBeEnabled();
    fireEvent.click(within(review).getByRole("button", { name: "Check again" }));
    expect(await within(review).findByText("Models available")).toBeVisible();
    expect(within(review).getByRole("button", { name: "Save connection" })).toBeEnabled();
    expect(mocks.validate).toHaveBeenCalledTimes(2);
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it("shows actionable setup diagnostics while collapsed and preserves CLI version labels", async () => {
    const grok = { ...grokConnection(), installedVersion: "grok 1.0.46 [alpha]" };
    const opencode = { ...defaultOpenCodeConnection(), installedVersion: "opencode v2.0.25", health: "setup_required" as const, error: null, message: "Configure a provider in OpenCode, then refresh." };
    mocks.list.mockResolvedValueOnce([grok, opencode]);
    render(<AiConnectionsSettings />);
    expect(await screen.findByText("grok 1.0.46 [alpha]")).toBeInTheDocument();
    expect(screen.getByText("opencode v2.0.25")).toBeInTheDocument();
    expect(screen.getByText("Sign in with the Grok CLI.")).toBeInTheDocument();
    expect(screen.getByText("Configure a provider in OpenCode, then refresh.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show Grok details" }));
    expect(screen.getAllByText("Sign in with the Grok CLI.")).toHaveLength(1);
  });

  it("keeps provider credentials out of the OpenCode wizard", async () => {
    render(<AiConnectionsSettings />);
    await screen.findByText("Local models");
    fireEvent.click(screen.getByRole("button", { name: "Add another" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByText(/Provider credentials stay in OpenCode/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/API key/i)).not.toBeInTheDocument();
  });

  it("offers Grok as a direct runtime without an API-key form", async () => {
    render(<AiConnectionsSettings />);
    await screen.findByText("Local models");
    fireEvent.click(screen.getByRole("button", { name: "Add another" }));
    fireEvent.click(screen.getByRole("button", { name: /Grok.*direct ACP coding-agent runtime/i }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByLabelText(/Grok executable/i)).toBeInTheDocument();
    expect(screen.getByText(/Authentication stays with Grok/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/API key/i)).not.toBeInTheDocument();
  });

  it("shows automatically registered standard runtimes without a connect step", async () => {
    mocks.list.mockResolvedValueOnce([codexConnection(), grokConnection(), defaultOpenCodeConnection()]);
    render(<AiConnectionsSettings />);
    expect(await screen.findByRole("article", { name: "Codex AI connection" })).toBeInTheDocument();
    expect(screen.getByRole("article", { name: "Grok AI connection" })).toHaveTextContent("setup required");
    expect(screen.getByRole("article", { name: "OpenCode AI connection" })).toHaveTextContent("unavailable");
    expect(screen.queryByRole("button", { name: "Connect Grok" })).not.toBeInTheDocument();
    expect(screen.getByText("3 provider connections")).toBeInTheDocument();
  });

  it("announces inventory failures without hiding the rest of Settings", async () => {
    mocks.list.mockRejectedValueOnce(new Error("OpenCode inventory unavailable"));
    render(<AiConnectionsSettings />);
    expect(await screen.findByRole("alert")).toHaveTextContent("OpenCode inventory unavailable");
    await waitFor(() => expect(screen.getByText("No AI provider connection yet")).toBeInTheDocument());
  });
});
