# Validation

Rustab has one canonical validation implementation: `scripts/validate`.

Use Nix to get the expected toolchain:

```sh
nix develop
```

or run non-interactively:

```sh
nix develop -c ./scripts/validate fast
```

## Modes

`scripts/validate` is the source of truth for the exact command sequence. The modes are named by intent:

| Mode | Command | Use it for | Primary failure class |
| --- | --- | --- | --- |
| `fast` | `./scripts/validate fast` | Normal source edits before commit or review. | Formatting, version metadata, JavaScript syntax/behavior, Rust lint/test failures. |
| `full` | `./scripts/validate full` | Release-sensitive local checks and packaging changes. | Everything from `fast`, plus Nix package/app/check and staging failures. |
| `release` | `./scripts/validate release` | Pre-tag or post-signing release sanity. | Everything from `full`, plus signed Firefox XPI/source mismatch or release-version mismatch. |

`fast` is secret-free and does not require refreshing the checked-in signed Firefox XPI. `full` exercises the Nix surface for the current system. `release` assumes the Firefox XPI has already been refreshed/signed and validates that release payload against source metadata.

When reporting failures, keep source failures, Nix packaging/environment failures, and signing/release-payload failures distinct.

## Command Menu

Inside the dev shell:

```sh
just --list
just fmt
just validate-fast
just validate-full
just test
```

`just` is only the ergonomic menu; keep substantive validation logic in `scripts/validate` so CI and agents use the same path.

## CI

Normal CI should call the same validation script instead of duplicating command lists:

```sh
nix develop -c ./scripts/validate full
```

Release automation derives tag and asset metadata with `scripts/check_versions.py --source-only --print-release-metadata`, runs `scripts/validate fast` before signing, refreshes/signs the Firefox XPI, then runs `scripts/validate release` because that mode intentionally validates the signed release payload.

## Formatting

`treefmt.toml` defines the polyglot formatting surface. To apply formatting:

```sh
treefmt
```

or:

```sh
just fmt
```

Validation uses `treefmt --fail-on-change`, so run formatting before committing.

## Jujutsu/Git Notes

Local Jujutsu state is allowed and encouraged for this repo, but Git remains the publication compatibility layer.

Useful local review commands:

```sh
jj status
jj diff
just review
```

Before publishing or tagging, also use Git-native checks:

```sh
git status --short --branch
git diff --check HEAD --
git log --oneline --decorate -5
```

## Known Failure Classes

- `nix flake check` can fail before compiling Rust if Nix cannot fetch/vendor crates from the network/cache. If `fast` passed, report that as packaging/environment failure, not as source-test failure.
- New files must be visible to the flake source snapshot. With Git alone this usually means staging/tracking them; with colocated Jujutsu this is calmer, but still verify flake/package checks before release-sensitive claims.
- Browser signing and store submission require secrets and should not be attempted in normal validation. The public boundary and current secret names are documented in `docs/PUBLIC_BOUNDARY.md`.
