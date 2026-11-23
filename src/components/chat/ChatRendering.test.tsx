import { act, render } from "@testing-library/react";
import { expect, it } from "vitest";
import { companionStore } from "@/lib/companionState";
import { disablePerformanceCounters, enablePerformanceCounters, takePerformanceCounters } from "@/lib/performanceCounters";
import { ConnectedChatTimeline } from "./ConnectedChatTimeline";

it("does not reproject or render the timeline when only the draft changes", () => {
  const repositoryId = 8021;
  companionStore.newConversation(repositoryId);
  enablePerformanceCounters();
  try {
    render(<ConnectedChatTimeline repositoryId={repositoryId} repositoryName="Render fixture" starters={[]} onStarter={() => {}} onCitation={() => {}} onError={() => {}} />);
    takePerformanceCounters();
    act(() => { for (let i = 0; i < 30; i++) companionStore.setDraft(repositoryId, `draft ${i}`); });
    expect(takePerformanceCounters()).toMatchObject({ timelineRenders: 0, timelineProjections: 0, modelPickerProjections: 0 });
  } finally { disablePerformanceCounters(); }
});
