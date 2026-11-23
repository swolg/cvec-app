# Third-party licenses and project assets

CVEC's own source is licensed under the [MIT license](LICENSE). Third-party dependencies, external tools, and trademarks remain governed by their respective licenses and terms.

## UI scaffold

Adapted UI primitives based on shadcn/ui retain the upstream MIT notice (`Copyright (c) 2023 shadcn`) in [LICENSES/shadcn-ui-MIT.txt](LICENSES/shadcn-ui-MIT.txt) ([source](https://github.com/shadcn-ui/ui/blob/main/LICENSE.md)).

## Dependencies and external tools

- Declared licenses for all locked npm and Cargo dependencies are tracked in [security/licenses/inventory.json](security/licenses/inventory.json) (`npm run licenses:check`).
- Notable non-MIT dependency licenses include **OFL-1.1** (Geist font), **MPL-2.0**, **Unicode-3.0**, and **CDLA-Permissive-2.0**.
- Syft, Grype, and optional AI CLIs (`Codex`, `Grok`, `OpenCode`) are installed separately by the user and remain subject to their own licenses and provider terms.

## Provider marks

CVEC bundles unmodified OpenAI and OpenCode SVG marks locally to identify connections in the model picker. These marks belong to their respective owners and are not covered by CVEC's MIT license. CVEC is an independent integration with no endorsement implied.

- **OpenAI:** [light](src/assets/provider-logos/openai-on-light.svg) and [dark](src/assets/provider-logos/openai-on-dark.svg) (`OpenAI-black-monoblossom.svg` / `OpenAI-white-monoblossom.svg` from [OpenAI-Logos-2025.zip](https://cdn.openai.com/brand/OpenAI-Logos-2025.zip)). Subject to [OpenAI Brand Guidelines](https://openai.com/brand/#usage-terms).
- **OpenCode:** [light](src/assets/provider-logos/opencode-on-light.svg) and [dark](src/assets/provider-logos/opencode-on-dark.svg) ([light source](https://opencode.ai/brand/files/opencode-logo-transparent-light.svg) · [dark source](https://opencode.ai/brand/files/opencode-logo-transparent-dark.svg)). Subject to [OpenCode Brand Guidelines](https://opencode.ai/brand).
- **Grok & GitLab:** Represented with generic UI icons ([Grok brand guidelines](https://x.ai/legal/brand-guidelines) · [GitLab trademark guidelines](https://handbook.gitlab.com/handbook/marketing/brand-experience/trademark-guidelines/)).

## Project assets

- App icons and favicons are derived from the CVEC Chihuahua motif ([branding notes](assets/branding/chihuahua-ultramarine/README.md)).
- The onboarding illustration ([onboarding-hero.webp](src/assets/onboarding-hero.webp)) was generated for CVEC using OpenAI image generation ([OpenAI terms](https://openai.com/policies/terms-of-use/#content)).
- Tray status dots are drawn directly in [application code](src-tauri/src/lib.rs).
