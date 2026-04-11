# rustab

Browser tab management from the terminal. A Rust replacement for [brotab](https://github.com/balta2ar/brotab).

> **Note**: This project was built with [Claude Code](https://github.com/anthropics/claude-code). Architecture was carefully designed based on studying the Claude Chrome extension's native messaging implementation, brotab, and tabctl.

Particularly useful with AI coding tools like Claude Code — lets your AI assistant list, search, open, and close browser tabs programmatically.

```
$ rustab list
b.42    GitHub - rustab      https://github.com/user/rustab
b.99    Nix manual           https://nixos.org/manual/nix/stable/
f.12    Reddit               https://www.reddit.com
$ rustab list | grep Reddit | rustab close
```

## Features

- List, close, activate, and open browser tabs from the CLI
- Supports Chrome, Brave, Firefox, Chromium, Edge, Vivaldi, Zen, Opera
- Pipe-friendly: `rustab list | grep pattern | rustab close`
- TSV and JSON output formats
- Multiple concurrent browsers
- Linux and macOS native messaging support
- Nix/flake-native packaging

## Architecture

```
Browser extension  <--native messaging (stdio)-->  rustab-mediator  <--Unix socket-->  rustab CLI
```

Each browser instance gets its own mediator process and Unix socket at `/tmp/rustab-{user}/{browser}-{pid}.sock`. The CLI discovers mediators by scanning this directory and filtering out stale sockets (dead PIDs).

Tab IDs are prefixed by browser: `c.123` (Chrome), `b.456` (Brave), `f.789` (Firefox), etc.

## Installation

### Nix / Home Manager

Add rustab as a flake input:

```nix
{
  inputs.rustab.url = "github:carjorvaz/rustab";
  inputs.rustab.inputs.nixpkgs.follows = "nixpkgs";
}
```

The flake provides four packages:
- `rustab` -- CLI + mediator binaries with native messaging manifests
- `chrome-extension` -- unpacked Chromium extension directory
- `firefox-extension` -- AMO-signed XPI for Firefox
- `package-chromium-release` -- helper for building a signed CRX + update feed bundle

The `rustab` package also exposes passthru metadata:
- `chromeExtension`
- `firefoxExtension`
- `chromeExtensionId`
- `firefoxExtensionId`

The flake `lib` output also provides:
- `chromeExtensionId`
- `firefoxExtensionId`
- `mkChromiumPolicy`

#### Brave / Chrome / Chromium

On Linux, a browser wrapper can load the unpacked extension via `--load-extension`:

```nix
# In your browser overlay or wrapper
"--load-extension=${inputs.rustab.packages.${system}.chrome-extension}"
```

Or, if you are using Home Manager's Chromium module:

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

On macOS, Chromium browsers still require a one-time `Load unpacked` step because fully declarative installation would need a packaged CRX and hosted update manifest. A clean approach is to expose the unpacked extension at a stable path in your home directory and load it once from `brave://extensions`.

### Managed Chromium Distribution

For fully declarative Chromium installation on macOS or managed Chromium installation on Linux, package a signed CRX and a static update manifest:

```sh
nix run .#package-chromium-release -- \
  --key /secure/path/rustab-chromium.pem \
  --base-url https://example.com/rustab/chromium
```

This produces:
- `rustab-<version>.crx`
- `updates.xml`
- `extension-settings.json`

The packaged release injects `update_url` into the staged manifest and signs the CRX with the provided private key, so future updates keep the same extension ID.

If you want `updates.xml` to live at one URL and the `.crx` to live somewhere else, pass an explicit codebase URL:

```sh
nix run .#package-chromium-release -- \
  --key /secure/path/rustab-chromium.pem \
  --base-url https://carjorvaz.github.io/rustab/chromium \
  --codebase-url https://github.com/carjorvaz/rustab/releases/download/v0.1.0/rustab-0.1.0.crx
```

That split is a good fit for GitHub Pages + GitHub Releases: keep `updates.xml` and `extension-settings.json` on Pages, keep the versioned CRX on Releases.

For managed Chromium browsers, host those files at the `base-url` you passed above and install with enterprise policy. In Nix, you can either write the policy yourself or use the helper from `inputs.rustab.lib`:

```nix
inputs.rustab.lib.mkChromiumPolicy {
  updateUrl = "https://example.com/rustab/chromium/updates.xml";
}
```

That returns:

```nix
{
  ExtensionSettings = {
    "<rustab-extension-id>" = {
      installation_mode = "force_installed";
      update_url = "https://example.com/rustab/chromium/updates.xml";
      override_update_url = true;
    };
  };
}
```

For Home Manager + Brave on Linux, that policy can be installed through the usual Chromium managed policy paths. For nix-darwin + Brave on macOS, serialize the same `ExtensionSettings` structure into `com.brave.Browser.plist` under `/Library/Managed Preferences/<user>/`.

#### Automated GitHub Releases + Pages

Rustab includes a tag-driven GitHub Actions workflow at `.github/workflows/release.yml` that automates the clean GitHub-hosted path:

- verifies that the pushed tag `vX.Y.Z` matches `extensions/chrome/manifest.json`
- runs tests and flake checks
- signs `rustab-<version>.crx`
- uploads the CRX to GitHub Releases
- deploys `updates.xml` and `extension-settings.json` to GitHub Pages under `/chromium/`

To use it:

1. Enable GitHub Pages for the repository with `GitHub Actions` as the source.
2. Add the repository secret `CHROMIUM_EXTENSION_KEY_PEM` containing the private key that matches the public key embedded in `extensions/chrome/manifest.json`.
3. Optionally set the repository variable `RUSTAB_CHROMIUM_BASE_URL` if you want a custom Pages or custom-domain URL. Otherwise the workflow defaults to `https://<owner>.github.io/<repo>/chromium`.
4. Push a tag like `v0.1.0`.

The workflow does not require a `gh-pages` branch. It deploys Pages directly from the workflow artifact, which keeps the repository history free of generated release files.

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
- **Chrome/Brave**: Go to `chrome://extensions`, enable Developer Mode, "Load unpacked" from `extensions/chrome/`
- **Firefox**: Open `extensions/firefox-signed/rustab@rustab.dev.xpi` in Firefox to install

`rustab install` uses the built-in Chromium extension ID by default. If you're testing a custom unpacked Chromium extension build with a different ID, pass `--chrome-extension-id <ID>`.

## Usage

```
rustab list                                # list all tabs (TSV)
rustab list --format json                  # list all tabs (JSON)
rustab list --browser brave                # list tabs from Brave only
rustab close b.42 b.99                     # close specific tabs
rustab list | grep github | rustab close   # pipe pattern
rustab activate c.42                       # focus a tab
rustab open https://example.com            # open URL in first available browser
rustab open -b firefox https://x.com       # open in specific browser
rustab clients                             # show connected browsers
```

## Development

The flake includes a dev shell with Rust toolchain and `web-ext` for Firefox extension signing:

```sh
# Sign the Firefox extension after changes (requires AMO API credentials in .web-ext-credentials)
source .web-ext-credentials
web-ext sign --source-dir=extensions/firefox --channel=unlisted \
  --api-key=$WEB_EXT_API_KEY --api-secret=$WEB_EXT_API_SECRET
cp web-ext-artifacts/*.xpi extensions/firefox-signed/rustab@rustab.dev.xpi
```

The Chromium release helper also works well from the dev shell:

```sh
nix develop -c package-chromium-release -- \
  --key /secure/path/rustab-chromium.pem \
  --base-url https://example.com/rustab/chromium
```

## License

AGPL-3.0-or-later
