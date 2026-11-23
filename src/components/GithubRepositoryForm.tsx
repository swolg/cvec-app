import { Plus } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";

import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/form-controls";

export interface GithubImportDraft {
  url: string;
  error: string | null;
}

interface GithubRepositoryFormProps {
  className?: string;
  disabled: boolean;
  disabledReason: string | null;
  focusToken?: number;
  draft?: GithubImportDraft;
  onDraftChange?: (draft: GithubImportDraft) => void;
  accountAction?: { label: string; onOpen: () => void };
  onAdd: (url: string) => Promise<void>;
  onCancel?: () => void;
  onSuccess?: () => void;
  submitLabel: string;
  submitVariant?: "default" | "secondary";
}

export function GithubRepositoryForm({
  className = "",
  disabled,
  disabledReason,
  focusToken = 0,
  draft,
  onDraftChange,
  accountAction,
  onAdd,
  onCancel,
  onSuccess,
  submitLabel,
  submitVariant = "default",
}: GithubRepositoryFormProps) {
  const [localDraft, setLocalDraft] = useState<GithubImportDraft>({ url: "", error: null });
  const { url, error: githubError } = draft ?? localDraft;
  const changeDraft = onDraftChange ?? setLocalDraft;
  const [isAddingGithub, setIsAddingGithub] = useState(false);
  const githubInputRef = useRef<HTMLInputElement | null>(null);
  const disableGithubAdd = disabled || isAddingGithub;

  useEffect(() => {
    if (focusToken) githubInputRef.current?.focus();
  }, [focusToken]);

  async function handleAddGithub() {
    const trimmedUrl = url.trim();
    if (!trimmedUrl) {
      changeDraft({ url, error: "Enter a GitHub repository URL." });
      return;
    }
    if (disableGithubAdd) {
      return;
    }

    setIsAddingGithub(true);
    changeDraft({ url, error: null });
    try {
      await onAdd(trimmedUrl);
      changeDraft({ url: "", error: null });
      onSuccess?.();
    } catch (cause) {
      changeDraft({ url, error: cause instanceof Error ? cause.message : String(cause) });
    } finally {
      setIsAddingGithub(false);
    }
  }

  return (
    <div className={className}>
      <div className="flex flex-col gap-2 sm:flex-row">
        <div className="flex min-w-0 flex-1 items-start gap-2">
          <Input
            ref={githubInputRef}
            aria-label="GitHub repository URL"
            className="h-10"
            containerClassName="min-w-0 flex-1"
            disabled={disableGithubAdd}
            disabledReason={disabledReason}
            error={githubError}
            onChange={(event) => changeDraft({ url: event.target.value, error: null })}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void handleAddGithub();
              }
              if (event.key === "Escape" && onCancel) {
                event.preventDefault();
                changeDraft({ url, error: null });
                onCancel();
              }
            }}
            placeholder="https://github.com/owner/repo"
            value={url}
          />
        </div>
        <Button
          className="rounded-lg"
          disabled={disableGithubAdd}
          onClick={() => void handleAddGithub()}
          type="button"
          variant={submitVariant}
        >
          <AppIcon className={isAddingGithub ? "size-4 animate-spin" : "size-4"} icon={Plus} />
          {isAddingGithub ? "Adding" : submitLabel}
        </Button>
      </div>
      {githubError && accountAction && /rate limit|sign in with github|private repositor|refused access|requires sso/i.test(githubError) ? (
        <Button className="mt-2" size="sm" variant="secondary" onClick={accountAction.onOpen} type="button">
          {accountAction.label}
        </Button>
      ) : null}
    </div>
  );
}
