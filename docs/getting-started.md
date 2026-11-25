# Getting started

Follow the [macOS build instructions](../README.md#build-and-run-on-macos) to build and run CVEC locally.

## First run

1. **Choose a repository folder:** Open **Repositories → Choose folder** (or **Settings → General → Local repositories**) and select a local Git repository or parent folder (defaults to `~/code`).
2. **Run a scan:** Make sure `syft` and `grype` are installed, then choose **Scan all**. The first run downloads the Grype vulnerability database and can take a moment.
3. **Review findings:** Open **Review** to inspect affected packages, remediation evidence, and mark findings as reviewed or snoozed. Closing the window keeps CVEC running in the menubar.
4. **Public advisories (optional):** Open **Intelligence**, pick watched topics, and choose **Fetch advisories** to compare public advisories against packages found in your local scans.
5. **Private GitHub repos (optional):** Follow the [GitHub connection guide](github-connection.md) to connect your own GitHub App, or clone repositories locally under your watched folder.
6. **AI chat and Fix workspaces (optional):** Configure Codex, Grok, or OpenCode in [AI connections](ai-connections.md). Try [Fix workspaces](fix-workspaces.md) on a clean, disposable Git repository first and review diffs and rescan results before applying changes.

## Scan builds and SBOMs

Select a repository and open **Artifacts → Choose and scan**. Select a built file, build folder, macOS app bundle, Docker or OCI image archive, OCI layout, or pre-generated Syft, CycloneDX or SPDX SBOM. Use the artifact produced by the same CI build you deploy. CVEC saves the input fingerprint, package count and Grype findings in the artifact history. Container results cover the platform selected by Syft and the image's final filesystem. SBOM results depend on the package evidence in the supplied file.

CVEC scans a temporary local copy of the selected input. Build files and folders support up to 8 GiB, and SBOM files support up to 128 MiB. Artifact scans have their own history alongside repository source scans.

## Known limitations

- **macOS first:** Targets macOS 12+; native testing has focused on Apple Silicon.
- **Updating:** Quit CVEC from the menubar, [back up your app data](local-data.md#back-up-your-app-data), pull the latest `main`, and rebuild.
- **Scanner coverage** includes resolved package versions, JavaScript development dependencies, and every severity reported by Grype. Rust checks development, build and target dependencies against `Cargo.lock`. Python checks development groups, optional extras and package versions in supported lockfiles. Repositories and Scan history show gaps in package coverage.
- **NuGet coverage** shows .NET projects and declared package requirements in **Dependencies**, including versions declared in a nearby `Directory.Packages.props`. Vulnerability matching uses resolved versions from lockfiles, legacy `packages.config`, restore metadata or scanned builds. For local scans, `dotnet restore` generates `obj/project.assets.json` for the checkout.
- **Match evidence** in Review includes the package version, matching method, and location when available. Go binary scans use function symbols when the scanner can read them. Review flags CPE matches, embedded workflows, JavaScript versions found only in a local installation, and matches based solely on Go module versions for verification before choosing a fix.
- **Transitive Cargo exceptions:** A small set of Linux-only or unmaintained transitive Cargo notices have documented exceptions; see [dependency assurance](dependency-assurance.md).
- **AI runtimes:** Analysis (read-only) and Fix workspaces have different requirements; see [runtime compatibility](fix-workspaces.md#runtime-compatibility). Live Grok Fix behavior is currently unverified beyond automated adapter tests.

## Feedback

Report reproducible bugs via the repository's issue form with your commit SHA, macOS/tool versions, and reproduction steps. Redact private repo names, source code, and credentials before attaching logs or screenshots.

For suspected security vulnerabilities, use the [private security reporting channel](../SECURITY.md).
