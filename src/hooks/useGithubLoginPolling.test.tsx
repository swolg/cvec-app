import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { pollGithubLogin } from "@/lib/api";
import type { GithubLoginPollResult, GithubLoginStart } from "@/lib/types";
import { useGithubLoginPolling } from "./useGithubLoginPolling";

vi.mock("@/lib/api", () => ({
  pollGithubLogin: vi.fn(),
}));

const login: GithubLoginStart = {
  loginId: "login-1",
  userCode: "ABCD-EFGH",
  verificationUri: "https://github.com/login/device",
  expiresAt: "2026-08-11T11:00:00Z",
  intervalSeconds: 3,
};

const pendingResult: GithubLoginPollResult = {
  status: "pending",
  message: null,
  intervalSeconds: 3,
  auth: {
    configured: true,
    signedIn: false,
    login: null,
    avatarUrl: null,
    profileUrl: null,
    expiresAt: null,
    rateLimitRemaining: null,
    rateLimitLimit: null,
    rateLimitResetAt: null,
    installUrl: null,
    message: null,
  },
};

describe("useGithubLoginPolling", () => {
  beforeEach(() => vi.mocked(pollGithubLogin).mockReset());
  afterEach(() => {
    vi.useRealTimers();
  });

  it("never starts a second poll while the first request is unresolved", async () => {
    vi.useFakeTimers();
    let resolveFirst: (result: GithubLoginPollResult) => void = () => undefined;
    vi.mocked(pollGithubLogin).mockImplementationOnce(
      () => new Promise((resolve) => (resolveFirst = resolve)),
    );
    vi.mocked(pollGithubLogin).mockResolvedValue(pendingResult);

    renderHook(() =>
      useGithubLoginPolling(login, vi.fn(), vi.fn(), vi.fn()),
    );

    await act(() => vi.advanceTimersByTimeAsync(3_000));
    expect(pollGithubLogin).toHaveBeenCalledTimes(1);
    await act(() => vi.advanceTimersByTimeAsync(30_000));
    expect(pollGithubLogin).toHaveBeenCalledTimes(1);

    await act(async () => resolveFirst(pendingResult));
    await act(() => vi.advanceTimersByTimeAsync(3_000));
    expect(pollGithubLogin).toHaveBeenCalledTimes(2);
  });

  it("honors slow_down with real login state and stops after success", async () => {
    vi.useFakeTimers();
    const onAuthChange = vi.fn();
    const setError = vi.fn();
    vi.mocked(pollGithubLogin).mockResolvedValueOnce({ ...pendingResult, status: "slow_down", intervalSeconds: 8 });
    vi.mocked(pollGithubLogin).mockResolvedValueOnce({ ...pendingResult, status: "success" });
    renderHook(() => {
      const [current, setCurrent] = useState<GithubLoginStart | null>(login);
      useGithubLoginPolling(current, setCurrent, onAuthChange, setError);
    });
    await act(() => vi.advanceTimersByTimeAsync(3_000));
    await act(() => vi.advanceTimersByTimeAsync(7_999));
    expect(pollGithubLogin).toHaveBeenCalledTimes(1);
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(pollGithubLogin).toHaveBeenCalledTimes(2);
    await act(() => vi.advanceTimersByTimeAsync(30_000));
    expect(pollGithubLogin).toHaveBeenCalledTimes(2);
    expect(onAuthChange).toHaveBeenCalledTimes(2);
  });

  it("ignores a pending response after login cancellation", async () => {
    vi.useFakeTimers();
    let finish!: (value: GithubLoginPollResult) => void;
    vi.mocked(pollGithubLogin).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const onAuthChange = vi.fn();
    const setLogin = vi.fn();
    const setError = vi.fn();
    const hook = renderHook(({ current }) => useGithubLoginPolling(current, setLogin, onAuthChange, setError), { initialProps: { current: login as GithubLoginStart | null } });
    await act(() => vi.advanceTimersByTimeAsync(3_000));
    hook.rerender({ current: null });
    await act(async () => finish({ ...pendingResult, status: "success" }));
    await act(() => vi.advanceTimersByTimeAsync(30_000));
    expect(pollGithubLogin).toHaveBeenCalledTimes(1);
    expect(onAuthChange).not.toHaveBeenCalled();
    expect(setLogin).not.toHaveBeenCalled();
  });
});
