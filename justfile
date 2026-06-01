default:
    @just --list

fmt:
    treefmt

fmt-check:
    treefmt --fail-on-change

validate mode="fast":
    ./scripts/validate {{mode}}

validate-fast:
    ./scripts/validate fast

validate-full:
    ./scripts/validate full

validate-release:
    ./scripts/validate release

check-versions:
    python3 scripts/check_versions.py

js-check:
    node --check extensions/shared/background_core.js
    node --check extensions/chrome/background.js
    node --check extensions/chrome/background_core.js
    node --check extensions/firefox/background.js
    node --check extensions/firefox/background_core.js

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo nextest run --workspace

flake-check:
    nix flake check --print-build-logs

jj-status:
    jj status

review:
    jj diff --tool difft
