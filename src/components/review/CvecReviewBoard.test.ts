import { describe, expect, it } from "vitest";

import type { ReviewFinding } from "@/lib/types";
import type { ReviewLane } from "@/lib/reviewProjection";
import { canAcceptPointerReviewDrop, createReviewDragItem, createReviewLaneDropOperation } from "./CvecReviewBoard";

describe("createReviewLaneDropOperation", () => {
  const finding = { reviewState: "unreviewed" } as ReviewFinding;
  const reviewedLane: ReviewLane = {
    id: "reviewed",
    label: "Reviewed",
    mutable: true,
    reviewState: "reviewed",
    items: [],
    totalCount: 0,
  };

  it("derives the destination directly from the internal drag payload", () => {
    const dragTypes = new Set(Object.keys(createReviewDragItem("1:fp-1", finding)));

    expect(createReviewLaneDropOperation(reviewedLane)({ type: "root" }, dragTypes, ["move"])).toBe("move");
  });

  it("rejects the source lane, read-only lanes, external drags, and non-root insertion targets", () => {
    const dragTypes = new Set(Object.keys(createReviewDragItem("1:fp-1", finding)));
    const sourceLane = { ...reviewedLane, id: "unreviewed", reviewState: "unreviewed" as const };
    const readOnlyLane = { ...reviewedLane, mutable: false, reviewState: undefined };

    expect(createReviewLaneDropOperation(sourceLane)({ type: "root" }, dragTypes, ["move"])).toBe("cancel");
    expect(createReviewLaneDropOperation(readOnlyLane)({ type: "root" }, dragTypes, ["move"])).toBe("cancel");
    expect(createReviewLaneDropOperation(reviewedLane)({ type: "root" }, new Set(["text/plain"]), ["move"])).toBe("cancel");
    expect(createReviewLaneDropOperation(reviewedLane)({ type: "item", key: "other", dropPosition: "before" }, dragTypes, ["move"])).toBe("cancel");
  });

  it("accepts a board-owned pointer drag over a destination lane and rejects the source lane", () => {
    const pointerDrag = { sourceReviewState: "unreviewed" as const };
    const sourceLane = { ...reviewedLane, id: "unreviewed", reviewState: "unreviewed" as const };

    expect(canAcceptPointerReviewDrop(reviewedLane, pointerDrag)).toBe(true);
    expect(canAcceptPointerReviewDrop(sourceLane, pointerDrag)).toBe(false);
    expect(canAcceptPointerReviewDrop(reviewedLane, null)).toBe(false);
  });
});
