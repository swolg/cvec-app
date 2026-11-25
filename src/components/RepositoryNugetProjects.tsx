import { Badge } from "@/components/ui/badge";
import type { NugetProject } from "@/lib/types";

export function RepositoryNugetProjects({ projects }: { projects: NugetProject[] }) {
  return (
    <section aria-label=".NET projects" className="mb-4 min-w-0 rounded-lg border border-hairline bg-surface-1">
      <header className="border-b border-hairline px-4 py-3">
        <h3 className="text-sm font-semibold text-ink">.NET projects</h3>
        <p className="mt-1 text-xs text-ink-subtle">NuGet version requirements declared in the scanned project files.</p>
      </header>
      <div className="divide-y divide-hairline">
        {projects.map((project) => (
          <details key={project.path} open={projects.length === 1}>
            <summary className="cursor-pointer px-4 py-3 text-sm font-medium text-ink [overflow-wrap:anywhere]">
              {project.path}
              <Badge className="ml-2" variant={project.resolutionPath ? "outline" : "warning"}>
                {project.resolutionPath ? "Resolved inventory" : "Declarations only"}
              </Badge>
            </summary>
            <div className="space-y-2 px-4 pb-3 text-xs text-ink-subtle">
              {project.frameworks.length ? <p>{project.frameworks.join(" · ")}</p> : null}
              {project.resolutionPath ? <p className="[overflow-wrap:anywhere]">Resolved from <code>{project.resolutionPath}</code></p> : null}
              {project.declarations.length ? (
                <ul aria-label="Declared NuGet packages" className="max-h-72 divide-y divide-hairline overflow-y-auto rounded-md border border-hairline">
                  {project.declarations.map((item) => (
                    <li key={JSON.stringify(item)} className="grid gap-1 px-3 py-2 [overflow-wrap:anywhere] sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                      <span className="font-medium text-ink">{item.name}{item.conditional ? <span className="ml-2 font-normal text-ink-subtle">Conditional</span> : null}</span>
                      <div>
                        {item.requirement ? <code>{item.requirement}</code> : <span>Version determined during restore</span>}
                        {item.requirementPath && item.requirementPath !== project.path ? <p>{item.requirementPath}</p> : null}
                      </div>
                    </li>
                  ))}
                </ul>
              ) : <p>0 literal package declarations</p>}
            </div>
          </details>
        ))}
      </div>
    </section>
  );
}
