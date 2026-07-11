# rustab

Browser tab management from the terminal. A Rust replacement for [brotab](https://github.com/balta2ar/brotab).

Particularly useful with AI coding tools like Claude Code — lets your AI assistant list, search, open, and close browser tabs programmatically.

```
$ rustab list
b.18452.42    GitHub - rustab      https://github.com/user/rustab
b.18452.99    Nix manual           https://nixos.org/manual/nix/stable/
f.20881.12    Reddit               https://www.reddit.com
$ rustab list | grep Reddit | rustab close
```

## Features

- List, close, move, activate, and open browser tabs from the CLI
- List browser windows and target tab operations by window
- Supports Chrome, Brave, Firefox, Chromium, Orion, Edge, Vivaldi, Zen
- List read-only synced Orion tabs from local macOS state
- Pipe-friendly: `rustab list | grep pattern | rustab close`
- TSV and JSON output formats
- Multiple concurrent browsers
- Linux and macOS native messaging support
- Nix/flake-native packaging

## Architecture

Rustab speaks to browser extensions through native messaging, with a small local mediator between the browser and the CLI. For runtime topology, source layout, identifier format, and maintainer-facing change boundaries, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

Treat tab and window IDs as opaque strings. Use the exact values printed by `rustab list` and `rustab windows`; scripts should not parse or synthesize them.

## Installation

### Nix / Home Manager

Add rustab as a flake input:

```nix
{
  inputs.rustab.url = "github:carjorvaz/rustab";
  inputs.rustab.inputs.nixpkgs.follows = "nixpkgs";
}
```

The flake exposes the CLI/native-host package and staged browser extension packages.

#### Brave / Chrome / Chromium

Install the native host package and load the staged Chromium extension package for your browser. For example, with Home Manager's Chromium-family modules:

```nix
let
  rustab = inputs.rustab.packages.${pkgs.stdenv.hostPlatform.system}.default;
in {
  home.packages = [ rustab ];

  programs.brave = {
    enable = true;
    nativeMessagingHosts = [ rustab ];
  };
}
```

On Linux, browser wrappers can also load the staged unpacked extension with `--load-extension=${inputs.rustab.packages.${system}.chrome-extension}`. On macOS, load the staged package once from the browser's extensions page, then use `rustab install` for native-host manifests.

#### Orion

On macOS, load the flake's `orion-extension` package with Orion's `Tools > Extensions > Install from Disk` flow, then run `rustab install` to write Orion's native-messaging host manifest.

`rustab synced list --browser orion` is read-only and uses Orion's local macOS sync cache.

#### Managed Chromium Distribution

For managed Chromium/enterprise installs, use the release helper instead of hand-assembling CRX/update-feed files:

```sh
nix run .#package-chromium-release -- --help
```

##### Automated GitHub Releases + Pages

Before pushing the first release tag, set the repository's **Settings → Pages → Source** to **GitHub Actions** and configure signing secrets by following [`docs/PUBLIC_BOUNDARY.md`](docs/PUBLIC_BOUNDARY.md). For a custom GitHub Pages or domain base URL, optionally set the `RUSTAB_CHROMIUM_BASE_URL` repository variable.

A `vX.Y.Z` tag matching the source metadata triggers [`.github/workflows/release.yml`](.github/workflows/release.yml). See [`docs/VALIDATION.md`](docs/VALIDATION.md) for validation and [`docs/PUBLIC_BOUNDARY.md`](docs/PUBLIC_BOUNDARY.md) for public-artifact boundaries and signing-secret rules.

#### Firefox / Zen

```nix
# home-manager
let
  rustab = inputs.rustab.packages.${pkgs.stdenv.hostPlatform.system}.default;
in
programs.firefox = {
  nativeMessagingHosts = [ rustab ];
  profiles.default.extensions.packages = [ rustab.firefoxExtension ];
};
```

### Manual

```sh
cargo build --release
./target/release/rustab install
```

Then load the browser extension:

For unpacked installs, load a staged extension package rather than a raw per-browser source directory:

```sh
nix build .#chrome-extension .#orion-extension
```

- **Chrome/Brave**: load `result` from `chrome://extensions` or `brave://extensions`
- **Orion**: load `result-1` in `Tools > Extensions > Install from Disk`
- **Firefox**: open the signed XPI from `extensions/firefox-signed/`

`rustab install` uses the built-in Chromium extension ID by default. If you are testing a custom unpacked Chromium extension build with a different ID, pass `--chrome-extension-id <ID>`.

## Usage

```
rustab list                                      # list all tabs (TSV)
rustab list --format json                        # list all tabs (JSON)
rustab list --browser brave                      # list tabs from Brave only
rustab windows                                   # list browser windows
rustab windows --format json                     # list windows as JSON
rustab synced list --browser orion               # list synced Orion tabs cached locally on macOS
rustab synced list --browser orion --archived    # inspect the newest non-empty archived Orion sync snapshot
rustab close <tab-id> <tab-id>                   # close tabs printed by rustab list
rustab list | grep github | rustab close         # pipe pattern
rustab move --to-window <window-id> <tab-id>     # move a tab to a window printed by rustab windows
rustab list | grep YouTube | rustab move --to-window <window-id>
rustab move --to-tab <tab-id> <tab-id>           # move a tab to the window containing another tab
rustab activate <tab-id>                         # focus a tab
rustab open https://example.com                  # open URL in the first responsive browser
rustab open -b firefox https://x.com             # open in a specific browser
rustab open --window <window-id> https://example.com
rustab clients                                   # show connected browsers, mediator PIDs, and sockets
rustab doctor                                    # diagnose manifests, mediators, and extension support
```

`rustab synced list` is intentionally read-only. Today it supports Orion on macOS and `--archived` is a debugging escape hatch for the newest non-empty backup snapshot.

Tab and window IDs are command tokens, not a public data model. Copy them from `rustab list` or `rustab windows` into later commands unchanged.

## Development

Durable repo guidance lives under `docs/`; start with `docs/README.md` and `AGENTS.md` if you are an agent.

For the canonical toolchain and exact validation modes, see `docs/VALIDATION.md`.

Quick source check:

```sh
nix develop -c ./scripts/validate fast
```

Release and packaging helpers are exposed as flake apps:

```sh
nix run .#refresh-firefox-xpi
nix run .#package-chromium-release -- --help
```

Use [`docs/VALIDATION.md`](docs/VALIDATION.md) for validation modes, [`.github/workflows/release.yml`](.github/workflows/release.yml) for automated release flow, and [`docs/PUBLIC_BOUNDARY.md`](docs/PUBLIC_BOUNDARY.md) for generated-artifact and signing-secret rules.

## License

AGPL-3.0-or-later
