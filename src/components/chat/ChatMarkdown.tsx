import { memo, useId, useMemo, useRef, type KeyboardEvent } from "react";
import ReactMarkdown from "react-markdown";
import { unified, type Processor } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import { openExternalUrl } from "@/lib/api";
import { countPerformanceRender, performanceCountersEnabled } from "@/lib/performanceCounters";
import { CopyButton } from "./CopyButton";

function trackMarkdownParsing(this: Processor) {
  const parse = this.parser;
  if (!parse) return;
  this.parser = (document, file) => {
    const started = performanceCountersEnabled() ? performance.now() : null;
    countPerformanceRender("markdownParses");
    countPerformanceRender("markdownParsedCharacters", document.length);
    const tree = parse(document, file);
    if (started !== null) countPerformanceRender("markdownParseDurationMs", performance.now() - started);
    return tree;
  };
}
const parser = unified().use(remarkParse).use(remarkGfm).use(trackMarkdownParsing);
type MarkdownTree = ReturnType<typeof parser.parse>;
interface Block { offset: number; text: string; tree: MarkdownTree | null }
interface Projection { source: string; streaming: boolean; deferred: boolean; sealed: Block[]; tailOffset: number; blocks: Block[] }

// Pure compiled Markdown only. Keep one loaded window reusable across remounts
// and abandoned React renders without retaining an unbounded history of ASTs.
const compiled = new Map<string, { projection: Projection; bytes: number }>();
const MAX_COMPILED_ENTRIES = 512;
const MAX_COMPILED_BYTES = 4 * 1024 * 1024;
let compiledBytes = 0;
export function clearMarkdownCache() { compiled.clear(); compiledBytes = 0; }
export function getMarkdownCacheMetrics() { return { entries: compiled.size, estimatedBytes: compiledBytes }; }

function rememberProjection(projection: Projection) {
  type Node = { type: string; children?: Node[] };
  const pending: Node[] = projection.blocks.flatMap((block) => block.tree ? [block.tree] : []);
  let nodes = 0;
  while (pending.length) {
    const node = pending.pop()!;
    nodes++;
    if (node.children) for (const child of node.children) pending.push(child);
  }
  const bytes = projection.source.length * 4 + nodes * 192 + projection.blocks.length * 128 + 512;
  if (bytes > MAX_COMPILED_BYTES) return;
  const prior = compiled.get(projection.source);
  if (prior) compiledBytes -= prior.bytes;
  compiled.delete(projection.source);
  compiled.set(projection.source, { projection, bytes });
  compiledBytes += bytes;
  while (compiled.size > MAX_COMPILED_ENTRIES || compiledBytes > MAX_COMPILED_BYTES) {
    const oldest = compiled.keys().next().value!;
    compiledBytes -= compiled.get(oldest)!.bytes;
    compiled.delete(oldest);
  }
}

function containsDefinitions(tree: MarkdownTree): boolean {
  type Node = { type: string; children?: Node[] };
  const pending: Node[] = [tree];
  while (pending.length) {
    const node = pending.pop()!;
    if (node.type === "definition" || node.type === "footnoteDefinition") return true;
    if (node.children) for (const child of node.children) pending.push(child);
  }
  return false;
}

export function projectMarkdown(source: string, previous?: Projection, streaming = false): Projection {
  if (previous?.source === source && !previous.deferred) return { ...previous, streaming };
  const reusable = previous && source.startsWith(previous.source) ? previous : undefined;
  // References can affect any earlier block, including definitions in quotes.
  const offset = reusable?.tailOffset ?? 0;
  const tail = source.slice(offset);
  if (streaming && tail.length > 64_000) {
    const sealed = offset ? reusable?.sealed ?? [] : [];
    return { source, streaming, deferred: true, sealed, tailOffset: offset, blocks: [...sealed, { offset, text: tail, tree: null }] };
  }
  let tree = parser.parse(tail);
  if (containsDefinitions(tree)) {
    if (offset) tree = parser.parse(source);
    return { source, streaming, deferred: false, sealed: [], tailOffset: 0, blocks: [{ offset: 0, text: source, tree }] };
  }
  const children = tree.children;
  const sealed = offset ? [...(reusable?.sealed ?? [])] : [];
  for (const child of children.slice(0, -1)) {
    const start = child.position?.start.offset ?? 0;
    const end = child.position?.end.offset ?? tail.length;
    sealed.push({ offset: offset + start, text: tail.slice(start, end), tree: { type: "root", children: [child] } });
  }
  const last = children.at(-1);
  const tailOffset = offset + (last?.position?.start.offset ?? 0);
  const final: Block = { offset: tailOffset, text: source.slice(tailOffset), tree: { type: "root", children: last ? [last] : [] } };
  return { source, streaming, deferred: false, sealed, tailOffset, blocks: [...sealed, final] };
}

export default function ChatMarkdown({ text, streaming = false }: { text: string; streaming?: boolean }) {
  const footnotePrefix = `user-content-${useId()}-`;
  const projection = useRef<Projection | undefined>(undefined);
  if (projection.current?.source !== text || projection.current.streaming !== streaming) {
    const cached = !streaming ? compiled.get(text) : undefined;
    if (cached) {
      compiled.delete(text); compiled.set(text, cached);
      countPerformanceRender("markdownCacheHits");
      projection.current = cached.projection;
    } else {
      projection.current = projectMarkdown(text, projection.current, streaming);
      if (!streaming) rememberProjection(projection.current);
    }
  }
  return <div className="min-w-0 max-w-full [overflow-wrap:anywhere]">{projection.current!.blocks.map((block) => <MarkdownBlock key={block.offset} block={block} footnotePrefix={footnotePrefix} />)}</div>;
}

const MarkdownBlock = memo(function MarkdownBlock({ block, footnotePrefix }: { block: Block; footnotePrefix: string }) {
  // React Markdown still owns HAST conversion, URL sanitization and safe JSX.
  // Its documented unified parser hook consumes our existing MDAST directly.
  const plugins = useMemo(() => [function reuseParsedTree(this: Processor) {
    this.parser = () => block.tree!;
  }], [block.tree]);
  // Identical footnote names in different answers must not point at each
  // other's content. Keep this rendering scope separate from the cached AST.
  const components = useMemo(() => ({
    ...markdownComponents,
    h2: (props: React.ComponentPropsWithoutRef<"h2">) => markdownComponents.h2({
      ...props, id: props.id === "footnote-label" ? `${footnotePrefix}footnote-label` : props.id,
    }),
    a: (props: React.ComponentPropsWithoutRef<"a">) => markdownComponents.a({
      ...props, "aria-describedby": props["aria-describedby"] === "footnote-label" ? `${footnotePrefix}footnote-label` : props["aria-describedby"],
    }),
  }), [footnotePrefix]);
  // Definitions require one document-wide tree. Its total length must not turn
  // otherwise small, safe blocks into plain text just because a link or
  // footnote was added. Keep the limit on individual top-level nodes instead.
  const oversized = block.text.length > 64_000 && block.tree?.children.some((node) =>
    (node.position?.end.offset ?? block.text.length) - (node.position?.start.offset ?? 0) > 64_000);
  return <div style={{ contentVisibility: "auto", containIntrinsicSize: "auto 100px" }}>
    {oversized || !block.tree ? <pre className="whitespace-pre-wrap break-words text-sm leading-6 text-ink">{block.text}</pre> : <ReactMarkdown remarkPlugins={plugins} remarkRehypeOptions={{ clobberPrefix: footnotePrefix }} components={components}>{block.text}</ReactMarkdown>}
  </div>;
});

function handleHorizontalScrollKey(event: KeyboardEvent<HTMLElement>) {
  // WKWebView can route arrow scrolling to the timeline despite region focus.
  if (event.target !== event.currentTarget || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const direction = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
  const node = event.currentTarget;
  if (!direction || node.scrollWidth <= node.clientWidth) return;
  event.preventDefault();
  node.scrollLeft += direction * 40;
}

function CodeBlock({ children }: React.ComponentPropsWithoutRef<"pre">) {
  const source = useRef<HTMLPreElement>(null);
  const text = () => source.current?.textContent ?? "";
  return <div className="my-3 min-w-0 max-w-full overflow-hidden rounded-md border border-hairline bg-surface-2">
    <div className="flex items-center justify-between gap-2 border-b border-hairline px-3 py-1"><span className="text-xs text-ink-subtle">Code</span><CopyButton label="Copy code" text={text} /></div>
    <pre ref={source} aria-label="Code block" role="region" tabIndex={0} onKeyDown={handleHorizontalScrollKey} className="min-w-0 max-w-full overflow-x-auto p-3 text-xs leading-5 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus [&_code]:bg-transparent [&_code]:p-0">{children}</pre>
  </div>;
}

const tableCellClassName = "min-w-28 border border-hairline px-3 py-2";

const markdownComponents = {
  table: ({ children }: React.ComponentPropsWithoutRef<"table">) => <div className="my-3 min-w-0 max-w-full overflow-x-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus" role="region" aria-label="Response table" tabIndex={0} onKeyDown={handleHorizontalScrollKey}><table className="w-full border-collapse text-left text-sm">{children}</table></div>,
  th: ({ children }: React.ComponentPropsWithoutRef<"th">) => <th className={`${tableCellClassName} bg-surface-2 font-semibold`}>{children}</th>,
  td: ({ children }: React.ComponentPropsWithoutRef<"td">) => <td className={`${tableCellClassName} align-top`}>{children}</td>,
  a: ({ href, children, ...props }: React.ComponentPropsWithoutRef<"a">) => <a {...props} className="font-medium text-primary underline decoration-primary/40 underline-offset-2 hover:decoration-primary" href={href} onClick={(event) => { if (!href || href.startsWith("#")) return; event.preventDefault(); void openExternalUrl(href); }} rel="noreferrer">{children}</a>,
  blockquote: ({ children }: React.ComponentPropsWithoutRef<"blockquote">) => <blockquote className="my-3 border-l-2 border-hairline-strong pl-3 text-ink-subtle">{children}</blockquote>,
  code: ({ children, className }: React.ComponentPropsWithoutRef<"code">) => <code className={`${className ?? ""} rounded bg-surface-3 px-1 py-0.5 font-mono text-[0.85em] text-ink`}>{children}</code>,
  h1: ({ children }: React.ComponentPropsWithoutRef<"h1">) => <h1 className="mb-2 mt-5 text-lg font-semibold tracking-tight text-ink first:mt-0">{children}</h1>,
  h2: ({ children, id, className }: React.ComponentPropsWithoutRef<"h2">) => <h2 id={id} className={`mb-2 mt-5 text-base font-semibold text-ink first:mt-0 ${className ?? ""}`}>{children}</h2>,
  h3: ({ children }: React.ComponentPropsWithoutRef<"h3">) => <h3 className="mb-1.5 mt-4 text-sm font-semibold text-ink first:mt-0">{children}</h3>,
  li: ({ children, id }: React.ComponentPropsWithoutRef<"li">) => <li id={id} className="pl-0.5">{children}</li>,
  ol: ({ children }: React.ComponentPropsWithoutRef<"ol">) => <ol className="my-2 list-decimal space-y-1 pl-5">{children}</ol>,
  p: ({ children }: React.ComponentPropsWithoutRef<"p">) => <p className="my-2 break-words text-sm leading-6 first:mt-0 last:mb-0">{children}</p>,
  pre: CodeBlock,
  strong: ({ children }: React.ComponentPropsWithoutRef<"strong">) => <strong className="font-semibold text-ink">{children}</strong>,
  ul: ({ children }: React.ComponentPropsWithoutRef<"ul">) => <ul className="my-2 list-disc space-y-1 pl-5">{children}</ul>,
};
