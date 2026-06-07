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

### `fast`

Normal pre-commit/source gate:

```sh
./scripts/validate fast
```

Runs:

1. `treefmt --fail-on-change`
2. `python3 scripts/check_versions.py --source-only`
3. `node --check` for all extension JavaScript entrypoints/shared code
4. `cargo clippy --workspace --all-targets -- -D warnings`
5. `cargo nextest run --workspace`

Use this for ordinary Rust, script, docs, manifest, and extension-source edits. It deliberately does not require the checked-in signed Firefox XPI to be refreshed; that belongs to the release path.

### `full`

Release-sensitive local gate:

```sh
./scripts/validate full
```

Runs `fast`, then:

```sh
nix flake check --print-build-logs
```

This verifies Nix packages/apps/checks for the current system. It may expose network/cache/vendor staging issues that are separate from source correctness; keep those failures distinct in reports.

### `release`

Pre-tag sanity gate:

```sh
./scripts/validate release
```

Runs `full`, then validates the checked-in signed Firefox XPI against the Firefox extension source and prints the source version from `scripts/check_versions.py --source-only --print-version`. Before pushing a tag, manually verify the intended annotated Git tag matches this version.

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

`.github/workflows/ci.yml` should call the same validation script instead of duplicating command lists:

```sh
nix develop -c ./scripts/validate fast
nix flake check --print-build-logs
```

Release automation may still run selected release-only packaging/signing steps separately because those require GitHub secrets.

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
difft git
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
- Browser signing and store submission require secrets and should not be attempted in normal validation.
