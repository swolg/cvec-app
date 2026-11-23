import {countPerformanceRender,recordPerformanceCommit} from "@/lib/performanceCounters";
import {
  ChatCircleDots,
  GitBranch,
} from "@phosphor-icons/react";
import { useContext, useEffect, useEffectEvent, useRef, useState, Profiler } from "react";

import { RepositoryChat } from "@/components/RepositoryChat";
import { CompactConversationSelector, ConversationRail } from "@/components/chat/ConversationRail";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/feedback";
import { companionScanStatus } from "@/lib/companionApi";
import { EvidenceReadsContext } from "@/lib/revisionReadCache";
import { companionStore, useCompanionSelector } from "@/lib/companionState";
import type { FindingChatRequest, RepositoryChatScanStatus, RepoSummary, ReviewOpenRequest } from "@/lib/types";

interface ChatWorkspaceProps {
  repos: RepoSummary[];
  selectedRepoId: number | null;
  findingRequest?: FindingChatRequest;
  onClearFinding?: () => void;
  onSelectRepo: (repoId: number) => void;
  onOpenReview: (request: ReviewOpenRequest) => void;
  onOpenAiConnections?: () => void;
  onOpenRepository?: (repoId: number) => void;
  onChooseLocalCheckout?: () => void;
  onOpenScannerSettings?: () => void;
  scannerUnavailable?: string | null;
  onError: (message: string) => void;
}

export function ChatWorkspace({
  repos,
  selectedRepoId,
  findingRequest,
  onClearFinding,
  onSelectRepo,
  onOpenReview,
  onOpenAiConnections,
  onOpenRepository,
  onChooseLocalCheckout,
  onOpenScannerSettings,
  scannerUnavailable,
  onError,
}: ChatWorkspaceProps) {
  const cache = useContext(EvidenceReadsContext)?.chatScan;
  const [analysisRequestKey, setAnalysisRequestKey] = useState(0);
  const [focusComposerRequest, setFocusComposerRequest] = useState<number | null>(null);
  const composerFocusSequence = useRef(0);
  const [scanLoad, setScanLoad] = useState<{
    key: string;
    status: RepositoryChatScanStatus | null;
  } | null>(null);
  const selectedRepo = repos.find((repo) => repo.id === selectedRepoId) ?? null;
  const repositoryId = selectedRepo?.id;
  const evidenceRevision = selectedRepo?.evidenceRevision;
  const scanKey = JSON.stringify([repositoryId, evidenceRevision, selectedRepo?.lastScanStatus, selectedRepo?.lastScannedAt, selectedRepo?.lastRemoteCommitSha]);
  const reportError = useEffectEvent((cause: unknown) => {
    onError(cause instanceof Error ? cause.message : String(cause));
  });
  countPerformanceRender("railRenders");
  const companion = useCompanionSelector((state)=>({activeThreadId:selectedRepoId?state.repositories[selectedRepoId]?.activeThreadId??null:null,index:state.conversationIndex}),(a,b)=>a.activeThreadId===b.activeThreadId&&a.index===b.index);
  const activeThreadId=companion.activeThreadId;
  const indexedThreads=companion.index?.threads??[];

  useEffect(() => {
    void companionStore.initialize().catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!selectedRepoId && repos.length) onSelectRepo(repos[0].id);
  }, [onSelectRepo, repos, selectedRepoId]);

  useEffect(() => {
    if (!repositoryId) return;
    let active = true;
    const read = async () => {
      const next = await companionScanStatus(repositoryId);
      if (next.repositoryId !== repositoryId) throw new Error("Scan context belongs to a different repository.");
      return next;
    };
    const request = !cache || evidenceRevision === undefined ? read() : cache.read(scanKey, read);
    void request.then((next) => {
        if (!active) return;
        setScanLoad({ key: scanKey, status: next });
      })
      .catch((cause) => {
        if (!active) return;
        setScanLoad({ key: scanKey, status: null });
        reportError(cause);
      });
    return () => { active = false; };
  }, [repositoryId, evidenceRevision, scanKey, cache]);

  const currentScan = scanLoad?.key === scanKey ? scanLoad : null;
  function selectConversation(repositoryId: number, threadId: number) {
    setFocusComposerRequest(null);
    setAnalysisRequestKey((value) => value + 1);
    onSelectRepo(repositoryId);
    void companionStore.activateConversation(repositoryId, threadId).catch((cause) => onError(cause instanceof Error ? cause.message : String(cause)));
  }

  function newConversation(repositoryId: number) {
    setFocusComposerRequest(++composerFocusSequence.current);
    setAnalysisRequestKey((value) => value + 1);
    onSelectRepo(repositoryId);
    companionStore.newConversation(repositoryId);
  }

  const rail = (<ConversationRail
        activeThreadId={activeThreadId}
        repositories={repos}
        selectedRepositoryId={selectedRepoId}
        threads={indexedThreads}
        onNewConversation={newConversation}
        onSelectConversation={selectConversation}
        onSelectRepository={(repositoryId) => { setFocusComposerRequest(null); onSelectRepo(repositoryId); }}
      />);

  if (!repos.length) {
    return (
      <Card data-chat-workspace data-chat-state="empty" className="app-panel min-h-[32rem] gap-0 rounded-lg border py-0 shadow-none">
        <EmptyState icon={GitBranch} title="No repositories yet" description="Add or discover a repository before starting a repository-scoped chat." />
      </Card>
    );
  }

  return (
    <div data-chat-workspace className="grid min-h-0 min-w-0 flex-1 grid-cols-1 gap-3 xl:grid-cols-[260px_minmax(0,1fr)]">
      {import.meta.env.VITE_CVEC_PERF_PROFILER==="on"?<Profiler id="conversation-rail" onRender={recordPerformanceCommit}>{rail}</Profiler>:rail}

      <Card className="chat-conversation-panel app-panel flex min-h-0 min-w-0 flex-col gap-0 overflow-hidden rounded-lg border py-0 shadow-none">
        <CompactConversationSelector
          activeThreadId={activeThreadId}
          repositories={repos}
          selectedRepositoryId={selectedRepoId}
          threads={indexedThreads}
          onNewConversation={newConversation}
          onSelectConversation={selectConversation}
          onSelectRepository={onSelectRepo}
        />
        {!selectedRepo ? (
          <div data-chat-state="empty"><EmptyState icon={ChatCircleDots} title="Choose a repository" description="Select which repository this conversation may use as context." /></div>
        ) : (
          <RepositoryChat
            key={selectedRepo.id}
            analysisRequestKey={analysisRequestKey}
            focusComposerRequest={focusComposerRequest}
            contextStatus={currentScan?.status ? "ready" : currentScan ? "error" : "loading"}
            scanStatus={currentScan?.status ?? null}
            findingRequest={findingRequest?.repositoryId === selectedRepo.id ? findingRequest : undefined}
            onClearFinding={onClearFinding}
            onError={onError}
            onOpenAiConnections={onOpenAiConnections}
            onOpenRepository={onOpenRepository ? () => onOpenRepository(selectedRepo.id) : undefined}
            onChooseLocalCheckout={onChooseLocalCheckout}
            onOpenScannerSettings={onOpenScannerSettings}
            scannerUnavailable={scannerUnavailable}
            onOpenFinding={(fingerprint) => onOpenReview({
              repositoryId: selectedRepo.id,
              findingFingerprint: fingerprint,
              origin: { repositoryId: selectedRepo.id, section: "chat" },
            })}
            repository={selectedRepo}
          />
        )}
      </Card>
    </div>
  );
}
