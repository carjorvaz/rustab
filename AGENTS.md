# Agent Notes

- `rustab` is a Rust CLI plus browser native-messaging bridge for terminal-driven tab management. Treat browser extension metadata, native-host manifests, and release artifacts as part of the product, not as incidental packaging.
- Start with `docs/README.md` for the durable repo map. Keep this file short and stable.
- Use `nix develop` for the canonical toolchain. Inside the dev shell, `just --list` shows the command menu and `./scripts/validate fast` is the normal pre-commit gate.
- Validation modes are documented in `docs/VALIDATION.md`. Prefer `scripts/validate fast` for source changes and `scripts/validate full` before release-sensitive changes.
- This repo is a Jujutsu-on-Git pilot: prefer `jj` for local mutation/history editing and recovery, but keep Git/GitHub for remotes, CI, annotated release tags, and compatibility. Do not commit `.jj` state.
- Read `docs/PUBLIC_BOUNDARY.md` before touching browser signing, AMO/GitHub secrets, release bundles, logs, or generated artifacts.
- Use checked-in plans only as described in `docs/PLANS.md`; keep private operational notes and credentials outside this public repo.
