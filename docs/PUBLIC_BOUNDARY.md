# Public Boundary

Rustab is a public repository. Treat every committed file, CI log, issue, release asset, and copied command transcript as public.

## Never Commit

- Browser store credentials, AMO API keys, GitHub tokens, or `.web-ext-credentials`.
- Chromium extension private keys such as `*.pem` used for CRX signing.
- Raw browser profiles, cookies, account state, local sync databases, or browsing history exports.
- User-specific native-messaging manifests containing private home-directory layouts when they are not example fixtures.
- CI logs, command transcripts, or debug dumps that include credentials, tokens, full private paths that matter operationally, or raw browser/account data.
- Generated release bundles unless the repo intentionally tracks that artifact class. Today the checked-in signed Firefox XPI is intentional; Chromium CRX/update-feed bundles are release outputs, not normal source commits.

## Allowed Source Artifacts

- Rust crates and tests.
- Browser extension source manifests, background scripts, icons, and shared code.
- The intentionally checked-in AMO-signed Firefox XPI at `extensions/firefox-signed/rustab@rustab.dev.xpi`.
- Nix package/app/check definitions.
- Secret-free scripts and docs.
- Example snippets using placeholder domains, keys, IDs, and paths.

## Signing and Release Secrets

Release signing belongs in private secret stores or GitHub Actions secrets:

- `CHROMIUM_EXTENSION_KEY_PEM`
- `WEB_EXT_API_KEY`
- `WEB_EXT_API_SECRET`

When testing release helpers locally, use private paths outside the repository and avoid pasting command output that echoes secret material.

## Debugging Native Messaging

Prefer redacted diagnostics:

- browser name/family;
- manifest path existence and permissions;
- mediator PID/socket liveness;
- scoped tab/window counts;
- error categories and command names.

Avoid raw dumps of browser profile data, synced-tab databases, cookies, account identifiers, or complete browsing histories unless the user explicitly provides sanitized fixtures for a test.

## CI and Logs

Normal CI must be secret-free. Release CI may use secrets, but logs should prove only that the secret was present and used, never print the secret itself.

Before adding new workflows, logs, generated reports, or artifacts, ask: would this be safe if copied into a public issue or indexed by a search engine?
