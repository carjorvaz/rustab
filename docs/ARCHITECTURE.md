# Architecture

Rustab lets terminal tools inspect and control browser tabs through native messaging.

```text
rustab CLI <--Unix socket--> rustab-mediator <--stdio native messaging--> browser extension <--browser tabs/windows APIs--> browser
```

## Product Boundary

Rustab owns:

- the `rustab` CLI and pipe-friendly command behavior;
- the mediator process that bridges local Unix sockets to browser native messaging;
- browser extension manifests and background scripts for Chromium-family browsers and Firefox-family browsers;
- Nix packages/apps/checks for installing the CLI, native hosts, and extension artifacts;
- release helpers for the signed Firefox XPI and managed Chromium CRX/update-feed path.

Rustab does **not** own:

- browser account credentials, sync accounts, cookies, profiles, or browsing data;
- browser store credentials or private signing keys;
- remote hosting of user-specific release artifacts;
- live browser state beyond what the user explicitly asks the CLI to list or mutate.

## Runtime Topology

1. The browser extension starts a native-messaging connection to `rustab-mediator`.
2. The mediator exposes a Unix socket under `/tmp/rustab-{uid}/` with a browser/pid-scoped name.
3. The CLI discovers mediator sockets, filters stale entries, and sends tab/window commands to responsive mediators.
4. The extension performs browser API calls and returns structured responses through the mediator.

Each browser instance has its own mediator. Scripts should prefer scoped IDs from `rustab list` and `rustab windows` because raw browser IDs are only unambiguous when a single matching browser instance is connected.

## Identifier Model

- Tab IDs use `browser.pid.tab`, for example `b.18452.42`.
- Window IDs use `browser.pid.w.window`, for example `b.18452.w.7`.
- Legacy two-part tab IDs such as `b.42` are compatibility input only and require an unambiguous connected browser instance.

Preserve scoped IDs in user-facing examples and tests unless a test is explicitly covering legacy compatibility.

## Source Layout

- `crates/rustab-cli/` — command-line parsing, socket discovery, tab/window operations, install and doctor commands.
- `crates/rustab-mediator/` — native-messaging bridge between browser stdio and local Unix sockets.
- `crates/rustab-protocol/` — shared request/response types for CLI, mediator, and browser extension messages.
- `extensions/shared/` — browser-independent extension behavior.
- `extensions/chrome/` — Chromium-family Manifest V3 manifest/background entrypoint and assets.
- `extensions/orion/` — Orion Manifest V2 persistent-background entrypoint and assets.
- `extensions/firefox/` — Firefox-family manifest/background entrypoint and assets.
- `extensions/firefox-signed/` — checked-in AMO-signed XPI consumed by the Nix package and release workflow.
- `scripts/` — validation and release-support scripts.
- `.github/workflows/` — GitHub Actions CI and release automation.

## Change Boundaries

- Protocol changes usually need synchronized updates across `rustab-protocol`, CLI/mediator handling, extension code, tests, and docs.
- Extension manifest version changes must stay synchronized with `Cargo.toml` and both browser manifests. Use `scripts/check_versions.py` or `nix run .#check-version-sync`.
- Firefox extension source changes require re-signing or refreshing `extensions/firefox-signed/rustab@rustab.dev.xpi` before release-grade flake checks can prove the packaged XPI matches the source version.
- Chromium managed-release changes must preserve the extension ID derived from the private key/public manifest key relationship; never commit private keys.

## Compatibility Commitments

- Keep normal source validation secret-free and runnable on macOS and Linux.
- Keep GitHub/Git compatibility even when local contributors use Jujutsu.
- Keep Nix flake outputs usable for package consumers; add new source files to version control before trusting flake/package checks.
