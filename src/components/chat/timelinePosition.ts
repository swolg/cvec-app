interface TimelinePosition {
  following: boolean;
  scrollTop: number;
  anchorId?: string;
  anchorOffset: number;
  visibleLimit: number;
}

// Small presentation metadata only; never retain message text or DOM nodes.
const positions = new Map<string, TimelinePosition>();

export function readTimelinePosition(key: string) {
  return positions.get(key);
}

export function saveTimelinePosition(key: string, node: HTMLElement, following: boolean, visibleLimit: number) {
  const top = node.getBoundingClientRect().top;
  const anchor = [...node.querySelectorAll<HTMLElement>("[data-message-id]")]
    .find((element) => element.getBoundingClientRect().bottom > top);
  positions.delete(key);
  positions.set(key, {
    following, visibleLimit, scrollTop: node.scrollTop,
    anchorId: anchor?.dataset.messageId,
    anchorOffset: anchor ? anchor.getBoundingClientRect().top - top : 0,
  });
  if (positions.size > 64) positions.delete(positions.keys().next().value!);
}

export function restoreTimelinePosition(key: string, node: HTMLElement) {
  const position = positions.get(key);
  if (!position || position.following) {
    node.scrollTop = node.scrollHeight;
    return;
  }
  node.scrollTop = position.scrollTop;
  const anchor = [...node.querySelectorAll<HTMLElement>("[data-message-id]")]
    .find((element) => element.dataset.messageId === position.anchorId);
  if (anchor) node.scrollTop += anchor.getBoundingClientRect().top - node.getBoundingClientRect().top - position.anchorOffset;
}
