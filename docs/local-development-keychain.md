# Local development and Keychain

Local macOS builds sign the app with a persistent self-signed certificate (**CVEC Local Development**) instead of a changing ad-hoc signature so macOS Keychain authorizations survive Rust rebuilds.

## One-time setup

```sh
npm run signing:setup
npm run signing:doctor
```

`signing:setup` creates the self-signed **CVEC Local Development** certificate in your login Keychain (valid for local code signing only, granted only to `/usr/bin/codesign`) and reuses it on later runs. No paid Apple Developer account is required.

- When macOS prompts for **codesign → CVEC Local Development**, approve it in the system dialog.
- If the first `npm run tauri -- dev` times out while waiting for that dialog, approve the prompt and rerun `npm run tauri -- dev`.
- Optionally run `npm run signing:smoke` to verify stable signing across temporary test builds.

## Running and building

```sh
npm run tauri -- dev
npm run tauri -- build --debug --bundles app
```

`src-tauri/tauri.macos.conf.json` configures `scripts/cargo-local.sh` and `scripts/run-signed.sh` to sign each rebuilt binary with bundle ID `dev.cvec.app` before launch. Set `APPLE_SIGNING_IDENTITY` (or pass `--config`) to override the certificate.

## Keychain behavior for GitHub sign-in

CVEC does not touch the Keychain on startup. After setting up [your GitHub App](github-connection.md), click **Use saved sign-in** in Settings or onboarding to load your saved token on demand, and choose **Always Allow** if macOS prompts for Keychain access. Public and local repositories can be scanned without signing in.
