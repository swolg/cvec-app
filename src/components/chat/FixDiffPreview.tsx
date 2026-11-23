import { useMemo, useState } from "react";
import type { FixDiffFile } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { CopyButton } from "./CopyButton";

const LINE_LIMIT = 2000;
type PatchLine = { text: string; kind: "metadata" | "hunk" | "addition" | "deletion" | "context"; oldLine: number | null; newLine: number | null };

function projectPatch(patch: string) {
  const source = patch.split("\n");
  if (source.at(-1) === "") source.pop();
  let oldLine = 1, newLine = 1, additions = 0, deletions = 0;
  let inHunk = false;
  const lines: PatchLine[] = [];
  for (const text of source) {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    let line: PatchLine;
    if (hunk) {
      inHunk = true;
      oldLine = Number(hunk[1]); newLine = Number(hunk[2]);
      line = { text, kind: "hunk", oldLine: null, newLine: null };
    } else if (text.startsWith("diff --git ")) { inHunk = false; line = { text, kind: "metadata", oldLine: null, newLine: null }; }
    else if (!inHunk && (text.startsWith("--- ") || text.startsWith("+++ "))) {
      // Untracked previews have headers but no @@ hunk. After +++, a line
      // resembling a header is payload and must retain its +/- count.
      inHunk = text.startsWith("+++ ");
      line = { text, kind: "metadata", oldLine: null, newLine: null };
    }
    else if (text.startsWith("+")) { additions++; line = { text, kind: "addition", oldLine: null, newLine: newLine++ }; }
    else if (text.startsWith("-")) { deletions++; line = { text, kind: "deletion", oldLine: oldLine++, newLine: null }; }
    else if (text.startsWith(" ")) line = { text, kind: "context", oldLine: oldLine++, newLine: newLine++ };
    else line = { text, kind: "metadata", oldLine: null, newLine: null };
    if (lines.length < LINE_LIMIT) lines.push(line);
  }
  return { lines, totalLines: source.length, additions, deletions };
}

export function FixDiffPreview({ file }: { file: FixDiffFile }) {
  const projected = useMemo(() => projectPatch(file.patch), [file.patch]);
  const [plain, setPlain] = useState(false);
  return <div className="min-w-0 border-t border-hairline">
    <div className="flex flex-wrap items-center justify-between gap-2 border-b border-hairline px-3 py-1">
      <span className="text-xs tabular-nums text-ink-subtle">+{projected.additions} / −{projected.deletions} lines{file.truncated ? " · truncated preview" : ""}</span>
      <CopyButton label={`Copy patch preview: ${file.path}`} text={file.patch} />
    </div>
    {plain ? <pre className="max-h-96 overflow-auto p-3 font-mono text-xs leading-5 text-ink">{file.patch}</pre> : <div role="region" aria-label={`Patch for ${file.path}`} tabIndex={0} className="max-h-96 overflow-auto py-2 font-mono text-xs leading-5 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus">
      {projected.lines.map((line, index) => <div key={index} data-diff-kind={line.kind} className={`grid min-w-max grid-cols-[3rem_3rem_minmax(0,1fr)] pr-3 ${line.kind === "addition" ? "bg-[color:var(--badge-success-bg)]" : line.kind === "deletion" ? "bg-destructive/10" : line.kind === "hunk" ? "bg-surface-3" : ""}`}>
        <span aria-hidden="true" className="select-none pr-2 text-right text-ink-subtle">{line.oldLine}</span><span aria-hidden="true" className="select-none border-r border-hairline pr-2 text-right text-ink-subtle">{line.newLine}</span><span className="whitespace-pre pl-3 text-ink">{line.text || " "}</span>
      </div>)}
    </div>}
    {projected.totalLines > LINE_LIMIT ? <div className="border-t border-hairline p-2 text-xs text-ink-subtle">Highlighted preview: first {LINE_LIMIT.toLocaleString()} of {projected.totalLines.toLocaleString()} lines. Copy includes the entire bounded patch preview.<Button className="ml-2" size="xs" variant="ghost" onClick={() => setPlain((value) => !value)}>{plain ? "Show highlighted preview" : "Show full plain patch"}</Button></div> : null}
  </div>;
}
