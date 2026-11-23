import { useState } from "react";
import { Button } from "@/components/ui/button";
import { loadSavedGithubAuth } from "@/lib/api";
import type { GithubAuthSnapshot } from "@/lib/types";

export function GithubSavedSignIn({ auth, onChange, onError }: {
  auth: GithubAuthSnapshot;
  onChange: (value: GithubAuthSnapshot) => void;
  onError: (value: string | null) => void;
}) {
  const [loading, setLoading] = useState(false);
  if (!auth.configured || auth.signedIn || (!auth.deferred && !auth.message)) return null;
  async function load() {
    setLoading(true);
    onError(null);
    try {
      const value = await loadSavedGithubAuth();
      onChange(value);
      if (value.message && !value.signedIn) onError(value.message);
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }
  return <Button disabled={loading || auth.checking} onClick={() => void load()} size="sm" type="button" variant="secondary">{loading ? "Checking saved sign-in…" : "Use saved sign-in"}</Button>;
}
