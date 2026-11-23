import {
  useEffect,
  useRef,
  type Dispatch,
  type SetStateAction,
} from "react";

import { pollGithubLogin } from "@/lib/api";
import type { GithubAuthSnapshot, GithubLoginStart } from "@/lib/types";

export function useGithubLoginPolling(
  login: GithubLoginStart | null,
  setLogin: Dispatch<SetStateAction<GithubLoginStart | null>>,
  onAuthChange: (auth: GithubAuthSnapshot) => void,
  setError: Dispatch<SetStateAction<string | null>>,
) {
  const onAuthChangeRef = useRef(onAuthChange);

  useEffect(() => {
    onAuthChangeRef.current = onAuthChange;
  }, [onAuthChange]);

  useEffect(() => {
    if (!login) {
      return;
    }

    let cancelled = false;
    let timeoutId: number | undefined;

    const schedulePoll = (intervalSeconds: number) => {
      timeoutId = window.setTimeout(async () => {
        try {
          const result = await pollGithubLogin(login.loginId);
          if (cancelled) {
            return;
          }

          onAuthChangeRef.current(result.auth);
          const terminal =
            result.status === "success" ||
            result.status === "expired" ||
            result.status === "denied";
          if (terminal) {
            setLogin(null);
          } else {
            if (result.status === "slow_down" && result.intervalSeconds > 0) {
              setLogin((current) =>
                current && current.loginId === login.loginId
                  ? { ...current, intervalSeconds: result.intervalSeconds }
                  : current,
              );
            }
            schedulePoll(Math.max(result.intervalSeconds, 3));
          }
          if (result.status !== "pending" && result.status !== "slow_down") {
            setError(result.message);
          }
        } catch (cause) {
          if (!cancelled) {
            setError(cause instanceof Error ? cause.message : String(cause));
            schedulePoll(Math.max(login.intervalSeconds, 3));
          }
        }
      }, Math.max(intervalSeconds, 3) * 1000);
    };

    schedulePoll(login.intervalSeconds);
    return () => {
      cancelled = true;
      if (timeoutId !== undefined) {
        window.clearTimeout(timeoutId);
      }
    };
  }, [login, setError, setLogin]);
}
