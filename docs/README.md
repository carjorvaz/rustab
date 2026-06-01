# Rustab Repo Map

This directory holds durable guidance for humans and agents working on rustab. Keep the top-level `AGENTS.md` short; put details here when they should survive chat history.

## Documents

- `ARCHITECTURE.md` — product boundary, process topology, identifiers, and source layout.
- `VALIDATION.md` — canonical local, CI, full, and release validation commands.
- `PUBLIC_BOUNDARY.md` — what must never be committed, logged, or exposed in this public repo.
- `PLANS.md` — when to create checked-in execution plans and what shape they should have.

## Command Surface

Enter the dev shell first:

```sh
nix develop
```

Then use the menu:

```sh
just --list
just validate-fast
just validate-full
```

For non-interactive runs, call the script directly through Nix:

```sh
nix develop -c ./scripts/validate fast
nix develop -c ./scripts/validate full
```

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
