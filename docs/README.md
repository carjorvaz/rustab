# Rustab Repo Map

This directory holds durable guidance for humans and agents working on rustab. Keep the top-level `AGENTS.md` short; put details here when they should survive chat history.

## Documents

- `ARCHITECTURE.md` — product boundary, process topology, source layout, and release/staging ownership.
- `VALIDATION.md` — canonical local, CI, full, and release validation guidance.
- `PUBLIC_BOUNDARY.md` — what must never be committed, logged, or exposed in this public repo.
- `PLANS.md` — when to create checked-in execution plans and what shape they should have.

## Validation, Release, and Staging

Use the Nix dev shell for the canonical toolchain and `just --list` for the command menu. Keep command details in `VALIDATION.md`; keep release and extension-staging ownership notes in `ARCHITECTURE.md` and `PUBLIC_BOUNDARY.md`.

## Tooling Posture

Rustab intentionally pilots a sharper agent-friendly stack while staying Git/GitHub compatible:

- Jujutsu (`jj`) for local mutation/history editing and recovery.
- Git/GitHub for remotes, CI, release tags, and broad compatibility.
- Difftastic for structural review when a text diff is noisy.
- `just` as the small command menu.
- `scripts/validate` as the canonical validation implementation.
- `treefmt` for polyglot formatting.
- `cargo-nextest` for the normal Rust test loop.
- Nix flakes for reproducible shells, packages, apps, and checks.

Tools should earn their place by making state, review, validation, or recovery clearer. Do not add rules or wrappers that duplicate another source of truth without improving feedback.
