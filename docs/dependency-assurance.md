# Dependency assurance

CVEC keeps its dependency supply chain tight by default:

- **Exact versions & no install scripts:** Direct production npm versions are exact, npm resolves only from `registry.npmjs.org`, CI installs with `--ignore-scripts`, and `.npmrc` disables lifecycle scripts.
- **Automated security gates:** CI runs `npm run dependency:check` (policy + SHA-pinned GitHub Actions), `npm run licenses:check`, `npm audit --omit=dev`, `cargo-audit` (pinned `0.22.2`), and OSV-Scanner (`2.7.0`, verified by SHA-256) against both lockfiles, plus CycloneDX 1.6 SBOM generation (`npm run sbom`).
- **Dependabot:** Proposes monthly npm, Cargo, and GitHub Actions updates after a 7-day cooldown (security updates bypass the cooldown). Major updates and packages with lifecycle scripts require maintainer review.
- **Reviewed exceptions:** Active transitive Cargo/OSV exceptions are listed in [security/dependency-policy.json](../security/dependency-policy.json) and explained in [dependency-exceptions.md](security/dependency-exceptions.md). Do not add or expand exceptions to silence a new finding without reviewing impact and removal conditions.

This repository currently provides read-only CI and secret-scanning workflows with no release-signing or publishing credentials.
