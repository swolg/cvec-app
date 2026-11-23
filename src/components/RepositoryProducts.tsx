import { useContext, useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { listRepositoryAssets } from "@/lib/api";
import type { RepoAsset } from "@/lib/types";
import { EvidenceReadsContext } from "@/lib/revisionReadCache";

const PRODUCT_NAMES: Readonly<Record<string, string>> = {
  "docker/docker": "Docker",
  "github/actions_runner": "GitHub Actions Runner",
  "github/enterprise_server": "GitHub Enterprise Server",
  "github/github": "GitHub",
  "gitlab/gitlab": "GitLab",
  "jenkins/jenkins": "Jenkins",
  "kubernetes/kubernetes": "Kubernetes",
  "nginx/nginx": "NGINX",
  "openssl/openssl": "OpenSSL",
  "postgresql/postgresql": "PostgreSQL",
};
const INITIAL_SOURCE_COUNT = 3;

export function RepositoryProducts({ repoId, evidenceRevision }: { repoId: number; evidenceRevision?: number }) {
  const cache = useContext(EvidenceReadsContext)?.products;
  const key = JSON.stringify([repoId, evidenceRevision]);
  const [load, setLoad] = useState<{ key: string; assets: RepoAsset[] | null }>();
  const current = load?.key === key ? load : undefined;
  const assets = current?.assets ?? null;
  const failed = current?.assets === null;
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    let active = true;
    const read = () => listRepositoryAssets(repoId);
    const request = !cache || evidenceRevision === undefined ? read() : cache.read(key, read);
    void request.then((result) => {
      if (active) setLoad({ key, assets: result });
    }).catch(() => {
      if (active) setLoad({ key, assets: null });
    });
    return () => { active = false; };
  }, [repoId, evidenceRevision, key, retry, cache]);

  const products = useMemo(() => {
    const groups = new Map<string, RepoAsset[]>();
    for (const asset of assets ?? []) {
      const key = JSON.stringify([asset.vendor, asset.product, asset.version]);
      const sources = groups.get(key);
      if (sources) sources.push(asset);
      else groups.set(key, [asset]);
    }
    return Array.from(groups);
  }, [assets]);

  return (
    <section className="rounded-lg border border-hairline bg-surface-1">
      <div className="border-b border-hairline px-4 py-3">
        <h3 className="text-sm font-semibold text-ink">Detected products</h3>
        <p className="mt-1 text-xs text-ink-subtle">Intelligence uses signals from repository files and Git remotes to match advisories.</p>
      </div>
      {failed ? (
        <div className="flex flex-wrap items-center justify-between gap-2 px-4 py-3">
          <p className="text-sm text-severity-critical" role="alert">Product signals could not be loaded.</p>
          <Button aria-label="Retry loading product signals" onClick={() => { setLoad(undefined); setRetry((attempt) => attempt + 1); }} size="xs" variant="secondary">Retry</Button>
        </div>
      ) : assets === null ? (
        <p className="px-4 py-3 text-sm text-ink-subtle" role="status">Loading product signals…</p>
      ) : products.length === 0 ? (
        <p className="px-4 py-3 text-sm text-ink-subtle">No product signals detected.</p>
      ) : null}
      {products.length > 0 ? (
        <ul className="divide-y divide-hairline px-4">
          {products.map(([key, sources]) => (
            <ProductSources key={key} sources={sources} />
          ))}
        </ul>
      ) : null}
    </section>
  );
}

function ProductSources({ sources }: { sources: RepoAsset[] }) {
  const [visibleCount, setVisibleCount] = useState(INITIAL_SOURCE_COUNT);
  const { vendor, product, version } = sources[0];
  const identity = `${vendor}/${product}`;
  const name = PRODUCT_NAMES[identity] ?? (vendor === product ? product : identity);
  const remaining = sources.length - visibleCount;
  const disclosureLabel = remaining > 0 ? "Show more sources" : "Show fewer sources";

  return (
    <li className="min-w-0 py-3">
      <p className="break-words text-sm font-medium text-ink">
        {name}{version ? <span className="ml-2 text-xs text-ink-subtle">{version}</span> : null}
      </p>
      <ul className="mt-1 space-y-1 text-xs text-ink-subtle">
        {sources.slice(0, visibleCount).map((source) => (
          <li className="break-all" key={JSON.stringify([source.assetKind, source.sourcePath, source.evidence])}>
            {source.sourcePath ? <code>{source.sourcePath}</code> : <span>{source.assetKind === "git_remote" ? "Git remote · " : ""}{source.evidence}</span>}
          </li>
        ))}
      </ul>
      {sources.length > INITIAL_SOURCE_COUNT ? (
        <Button
          aria-label={`${disclosureLabel} for ${name}`}
          className="mt-1 -ml-2"
          onClick={() => setVisibleCount(remaining > 0 ? visibleCount + 20 : INITIAL_SOURCE_COUNT)}
          size="xs"
          variant="ghost"
        >
          {disclosureLabel}{remaining > 0 ? ` (${remaining})` : ""}
        </Button>
      ) : null}
    </li>
  );
}
