# Public Boundary

Rustab is a public repository. Treat every committed file, CI log, issue, release asset, and copied command transcript as public.

## Never Commit

- Browser store credentials, AMO API keys, GitHub tokens, or `.web-ext-credentials`.
- Chromium extension private keys such as `*.pem` used for CRX signing.
- Raw browser profiles, cookies, account state, local sync databases, or browsing history exports.
- User-specific native-messaging manifests containing private home-directory layouts when they are not example fixtures.
- CI logs, command transcripts, or debug dumps that include credentials, tokens, full private paths that matter operationally, or raw browser/account data.
- Generated release bundles unless they match an explicit exception below.

## Generated Artifact Exceptions

The only generated release artifact intentionally tracked in source is the AMO-signed Firefox XPI at:

```text
extensions/firefox-signed/<gecko-id>.xpi
```

`<gecko-id>` comes from the Firefox manifest. Chromium CRX/update-feed bundles are release outputs, not normal source commits.

## Allowed Source Artifacts

- Rust crates and tests.
- Browser extension source manifests, background scripts, icons, and shared code.
- The generated artifact exception defined above.
- Secret-free scripts and docs, including staging helpers that materialize extension payloads from checked-in source files.
- Nix package/app/check definitions.
- Example snippets using placeholder domains, keys, IDs, and paths.

## Signing and Release Secrets

Release signing belongs in private secret stores or GitHub Actions secrets. Current repository secret names:

- `CHROMIUM_EXTENSION_KEY_PEM` — private key for managed Chromium CRX signing.
- `WEB_EXT_API_KEY` — AMO unlisted signing API key.
- `WEB_EXT_API_SECRET` — AMO unlisted signing API secret.

When testing release helpers locally, use private paths outside the repository and avoid pasting command output that echoes secret material. Other docs should link here instead of repeating generated-artifact exceptions or signing-secret lists.

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
