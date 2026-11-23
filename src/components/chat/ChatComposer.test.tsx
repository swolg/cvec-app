import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ProviderConnectionSnapshot } from "@/lib/types";

import { ChatComposer } from "./ChatComposer";
import { disablePerformanceCounters, enablePerformanceCounters, takePerformanceCounters } from "@/lib/performanceCounters";

function connection(id: string, name: string, model: string, efforts: string[], local = false): ProviderConnectionSnapshot {
  return {
    schemaVersion: 1,
    connection: { schemaVersion: 1, id, driver: id.startsWith("codex") ? "codex" : "open_code", displayName: name, accentColor: null, enabled: true, builtIn: true, config: id.startsWith("codex") ? { kind: "codex", binaryPath: null, homePath: null, profile: null, launchArgs: null } : { kind: "open_code", binaryPath: null, mode: "managed_local", serverUrl: null }, hasServerPassword: false, createdAt: "now", updatedAt: "now" },
    health: "ready", installedVersion: "1", authKind: id.startsWith("codex") ? "chatgpt" : "none", accountLabel: id.startsWith("codex") ? "dev@example.test" : null,
    capabilities: { createThread: true, resumeThread: true, interrupt: true, orderedStreaming: true, readOnlyPolicy: true, accountStatus: true, rateLimits: true, tokenUsage: true, reasoningEffort: true, attachments: true },
    upstreamProviders: [{ id: local ? "ollama" : "openai", displayName: local ? "Ollama" : "OpenAI", connected: true, inferenceBoundary: local ? "verified_local" : "external" }],
    models: [{ connectionId: id, driver: id.startsWith("codex") ? "codex" : "open_code", upstreamProviderId: local ? "ollama" : "openai", inferenceBoundary: local ? "verified_local" : "external", id: model, displayName: model, description: `${name} model`, isDefault: true, hidden: false, reasoningEfforts: efforts, inputModalities: ["text"], contextWindow: 10_000 }],
    checkedAt: "now", message: null, error: null,
  };
}

const codex = connection("codex_default", "Codex", "gpt-test", ["low", "medium", "high", "xhigh"]);
const local = connection("opencode_default", "OpenCode", "ollama/qwen", [], true);
const grok: ProviderConnectionSnapshot = {
  ...codex,
  connection: { ...codex.connection, id: "grok_default", driver: "grok", displayName: "Grok", config: { kind: "grok", binaryPath: null, customModels: [] } },
  authKind: "none",
  accountLabel: null,
  upstreamProviders: [{ id: "xai", displayName: "xAI", connected: true, inferenceBoundary: "external" }],
  models: [{ ...codex.models[0], connectionId: "grok_default", driver: "grok", upstreamProviderId: "xai", id: "grok-4.6", displayName: "Grok 4.6" }],
};
const openrouter: ProviderConnectionSnapshot = {
  ...local,
  connection: { ...local.connection, id: "opencode_cloud", displayName: "OpenCode Cloud" },
  upstreamProviders: [{ id: "openrouter", displayName: "OpenRouter", connected: true, inferenceBoundary: "external" }],
  models: Array.from({ length: 25 }, (_, index) => ({
    ...local.models[0],
    connectionId: "opencode_cloud",
    upstreamProviderId: "openrouter",
    inferenceBoundary: "external" as const,
    id: `openrouter/model-${index.toString().padStart(2, "0")}`,
    displayName: `Model ${index.toString().padStart(2, "0")}`,
  })),
};

function renderComposer(overrides: Partial<React.ComponentProps<typeof ChatComposer>> = {}) {
  const props: React.ComponentProps<typeof ChatComposer> = {
    draft: "Question", selection: { repositoryId: 1, connectionId: "codex_default", modelId: "gpt-test", effort: "low" },
    models: codex.models, connections: [codex, local], selectionLoading: false,
    threadUsage: { inputTokens: 2000, cachedInputTokens: 0, outputTokens: 500, reasoningTokens: 0, totalTokens: 2500, contextWindow: 10_000, contextUsed: 2500 },
    providerUsage: { thread: null, primary: { usedPercent: 10, windowDurationMinutes: 300, resetsAt: null }, secondary: null, lifetimeTokens: 12_000, peakDailyTokens: null, supported: true, unavailableReason: null, fetchedAt: "now" },
    contextControls: <button type="button">Context</button>, contextChips: <span>repo</span>, statusMessage: "Ready", placeholder: "Ask",
    disabled: false, sending: false, canSend: true, historyCachedOnly: false, retryableError: false,
    onDraftChange: vi.fn(), onSelectModel: vi.fn(), onEffortChange: vi.fn(), onOpenSettings: vi.fn(), onSend: vi.fn(), onRetry: vi.fn(), onStop: vi.fn(),
    ...overrides,
  };
  return { props, view: render(<ChatComposer {...props} />) };
}

describe("ChatComposer", () => {
  it("does not derive a closed model catalog while the user types", () => {
    enablePerformanceCounters();
    try {
      const { props, view } = renderComposer({ connections: [codex, openrouter] });
      takePerformanceCounters();
      view.rerender(<ChatComposer {...props} draft="A new security question" />);
      expect(takePerformanceCounters().modelPickerProjections).toBe(0);
    } finally { disablePerformanceCounters(); }
  });
  it("switches between connection-scoped model lists and keeps effort capability-specific", async () => {
    const { props, view } = renderComposer({ connections: [codex, grok, local] });
    const user = userEvent.setup();
    const modelPicker = screen.getByRole("button", { name: "AI connection and model" });
    expect(modelPicker).toHaveTextContent("gpt-test");
    expect(modelPicker).not.toHaveTextContent("Codex ·");
    expect(modelPicker.querySelector('[data-provider-icon="codex"]')).toBeInTheDocument();
    const reasoning = screen.getByRole("combobox", { name: "Reasoning effort" });
    expect(reasoning).toHaveTextContent("Low");
    const indicator = reasoning.querySelector('[data-reasoning-level="low"]');
    const bars = indicator?.querySelectorAll('[data-reasoning-indicator="bars"] [data-active]');
    expect(bars).toHaveLength(4);
    expect(bars?.[0]).toHaveStyle({ height: "4px" });
    expect(bars?.[3]).toHaveStyle({ height: "12px" });
    expect(indicator?.querySelectorAll('[data-active="true"]')).toHaveLength(1);

    await user.click(reasoning);
    await user.click(await screen.findByRole("option", { name: "High" }));
    expect(props.onEffortChange).toHaveBeenCalledWith("high");

    await user.click(screen.getByRole("button", { name: "AI connection and model" }));
    expect(document.querySelector("[data-model-viewport=true]")).toHaveClass("h-[19rem]");
    expect(screen.getByRole("button", { name: "Codex models" })).toHaveAttribute("aria-current", "true");
    for (const [name, driver, brand] of [
      ["Codex", "codex", "openai"],
      ["OpenCode", "open_code", "opencode"],
    ]) {
      const providerButton = screen.getByRole("button", { name: `${name} models` });
      const icon = providerButton.querySelector(`[data-provider-icon="${driver}"]`);
      expect(icon).toHaveAttribute("aria-hidden", "true");
      const images = icon?.querySelectorAll("img");
      expect(images).toHaveLength(2);
      expect(images?.[0]).toHaveAttribute("src", expect.stringContaining(`/provider-logos/${brand}-on-light.svg`));
      expect(images?.[0]).toHaveClass("dark:hidden");
      expect(images?.[1]).toHaveAttribute("src", expect.stringContaining(`/provider-logos/${brand}-on-dark.svg`));
      expect(images?.[1]).toHaveClass("hidden", "dark:block");
    }
    const grokIcon = screen.getByRole("button", { name: "Grok models" }).querySelector('[data-provider-icon="grok"]');
    expect(grokIcon).toHaveAttribute("aria-hidden", "true");
    expect(grokIcon?.querySelector("svg")).toBeInTheDocument();
    expect(grokIcon?.querySelector("img")).toBeNull();

    await user.click(screen.getByRole("button", { name: "OpenCode models" }));
    expect(screen.getByText("OpenCode · Ollama")).toBeInTheDocument();
    await user.click(screen.getByText("ollama/qwen"));
    expect(props.onSelectModel).toHaveBeenCalledWith("opencode_default", "ollama/qwen");
    expect(screen.queryByRole("dialog", { name: "Connection and model" })).not.toBeInTheDocument();

    view.rerender(<ChatComposer {...props} models={local.models} selection={{ repositoryId: 1, connectionId: "opencode_default", modelId: "ollama/qwen", effort: null }} />);
    expect(screen.queryByRole("combobox", { name: "Reasoning effort" })).not.toBeInTheDocument();
    const openCodePicker = screen.getByRole("button", { name: "AI connection and model" });
    expect(openCodePicker).toHaveTextContent("ollama/qwen");
    expect(openCodePicker).not.toHaveTextContent("OpenCode ·");
    expect(openCodePicker.querySelector('[data-provider-icon="open_code"]')).toBeInTheDocument();
  });

  it("keeps large provider catalogs in one fixed, searchable model list", async () => {
    renderComposer({ connections: [codex, openrouter] });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "AI connection and model" }));

    expect(screen.queryByText("Model 24")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "OpenCode Cloud models" }));
    expect(screen.getByText("Model 24")).toBeInTheDocument();
    expect(screen.queryByText(/Show more/)).not.toBeInTheDocument();
    expect(screen.getAllByText("OpenCode Cloud · OpenRouter")).toHaveLength(25);
    expect(document.querySelector("[data-model-viewport=true]")).toHaveClass("h-[19rem]");
    expect(document.querySelector("[data-model-list=true]")).toHaveClass("h-full", "overflow-y-auto");

    await user.clear(screen.getByRole("searchbox", { name: "Search AI models" }));
    await user.type(screen.getByRole("searchbox", { name: "Search AI models" }), "Model 24");
    expect(screen.getByText("Model 24")).toBeInTheDocument();
    expect(screen.queryByText("Model 00")).not.toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "AI connections" })).not.toBeInTheDocument();
  });

  it("distinguishes context-window consumption from provider quota and presents atomic loading", async () => {
    const { props, view } = renderComposer();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Context window 25 percent used" }));
    expect(screen.getByRole("progressbar", { name: "2,500 of 10,000 estimated context tokens used" })).toHaveAttribute("aria-valuenow", "2500");
    expect(screen.getByText("Current context estimate")).toBeInTheDocument();
    expect(screen.getByText(/lifetime thread usage is not treated as active context/i)).toBeInTheDocument();
    expect(screen.getByText("Provider usage")).toBeInTheDocument();

    view.rerender(<ChatComposer {...props} selectionLoading />);
    expect(screen.getByRole("button", { name: "AI connection and model" })).toHaveTextContent("Checking selection");
    expect(screen.queryByText("No model selected")).not.toBeInTheDocument();
  });

  it("shows a compact retry action without disabling normal editing", async () => {
    const { props } = renderComposer({ retryableError: true, statusMessage: "Provider history is unavailable." });
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(props.onRetry).toHaveBeenCalledOnce();
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toHaveValue("Question");
  });
});
