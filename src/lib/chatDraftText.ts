// Matches the profile-local draft storage limit, measured in UTF-16 units.
export const CHAT_DRAFT_MAX_LENGTH = 65_536;

export function limitDraftText(text: string, maxLength = CHAT_DRAFT_MAX_LENGTH): string {
  let end = Math.min(text.length, Math.max(0, Math.floor(maxLength)));
  const last = text.charCodeAt(end - 1);
  // Never retain the first half of an emoji when the limit splits its pair.
  if (last >= 0xd800 && last <= 0xdbff) end -= 1;
  // Preserve valid pairs; repair any other unpaired units before native JSON
  // decoding. The string remains within the same UTF-16 storage bound.
  return text.slice(0, end).replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]|[\uD800-\uDFFF]/g, (value) => value.length === 2 ? value : "\uFFFD");
}
