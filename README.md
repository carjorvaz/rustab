# rustab

Browser tab management from the terminal. A Rust replacement for [brotab](https://github.com/balta2ar/brotab).

> **Note**: This project was built with [Claude Code](https://github.com/anthropics/claude-code). Architecture was carefully designed based on studying the Claude Chrome extension's native messaging implementation, brotab, and tabctl.

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

```
Browser extension  <--native messaging (stdio)-->  rustab-mediator  <--Unix socket-->  rustab CLI
```

Each browser instance gets its own mediator process and Unix socket at `/tmp/rustab-{uid}/{browser}-{pid}.sock`. The CLI discovers mediators by scanning this directory and filtering out stale sockets (dead PIDs).

Rustab emits full tab IDs that include the browser prefix, mediator PID, and browser tab ID: `c.18452.123`, `b.20881.456`, `f.19001.789`, etc. The legacy two-part form (`c.123`) is still accepted when only one matching browser instance is connected.

Window IDs use the same scoped form with a `w` marker: `c.18452.w.12`, `b.20881.w.34`, etc. Raw browser window IDs are accepted by commands that target a window when only one browser instance is in play, but the scoped IDs from `rustab windows` are the safest form for scripts.

## Installation

### Nix / Home Manager

Add rustab as a flake input:

```nix
{
  inputs.rustab.url = "github:carjorvaz/rustab";
  inputs.rustab.inputs.nixpkgs.follows = "nixpkgs";
}
```

The flake provides six packages:
- `rustab` -- CLI + mediator binaries with native messaging manifests
- `chrome-extension` -- unpacked Chromium extension directory
- `firefox-extension` -- AMO-signed XPI for Firefox
- `check-version-sync` -- helper for verifying release metadata stays aligned
- `refresh-firefox-xpi` -- helper for re-signing and refreshing the checked-in Firefox XPI
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

#### Brave / Chrome / Chromium / Orion

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

On macOS, Chromium browsers still require a one-time manual extension install because fully declarative installation would need a packaged CRX and hosted update manifest. A clean approach is to expose the unpacked extension at a stable path in your home directory and load it once from `brave://extensions`, `chrome://extensions`, or Orion's `Tools > Extensions > Install from Disk`.

Rustab also installs the native messaging host manifest for Brave into Chromium-family fallback locations on macOS. This is intentional: current Brave releases do not always discover `NativeMessagingHosts` from their branded `BraveSoftware/Brave-Browser` application-support directory, but they do reliably pick up the standard Chromium user paths.

That means `rustab install` may report multiple manifest locations for a single Brave profile on macOS. This is expected.

On macOS, `rustab install` also writes Orion's native messaging host manifest to `~/Library/Application Support/Orion/NativeMessagingHosts`.

### Managed Chromium Distribution

For managed Chromium installation on Linux, or for enterprise-managed Chromium installation on macOS, package a signed CRX and a static update manifest:

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
  --codebase-url https://github.com/carjorvaz/rustab/releases/download/vX.Y.Z/rustab-X.Y.Z.crx
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

For Home Manager + Brave on Linux, that policy can be installed through the usual Chromium managed policy paths. For nix-darwin + Brave on macOS, serialize the same `ExtensionSettings` structure into `com.brave.Browser.plist` under `/Library/Managed Preferences/<user>/`, but only expect off-store self-hosted installs to work when the browser is enterprise-managed. On unmanaged macOS Brave, the supported Rustab path remains a one-time `Load unpacked` step for the extension plus the declarative native-host setup described above.

#### Automated GitHub Releases + Pages

Rustab includes a tag-driven GitHub Actions workflow at `.github/workflows/release.yml` that automates the clean GitHub-hosted path:

- verifies that Cargo, Chromium, Firefox, and the committed signed Firefox XPI all agree on `X.Y.Z`, and that the signed Firefox XPI still matches the checked-in Firefox extension source
- runs tests and flake checks
- signs `rustab-<version>.crx`
- signs `rustab@rustab.dev.xpi`
- uploads both browser artifacts to GitHub Releases
- deploys `updates.xml` and `extension-settings.json` to GitHub Pages under `/chromium/`

To use it:

1. Enable GitHub Pages for the repository with `GitHub Actions` as the source.
2. Add the repository secret `CHROMIUM_EXTENSION_KEY_PEM` containing the private key that matches the public key embedded in `extensions/chrome/manifest.json`.
3. Add the repository secrets `WEB_EXT_API_KEY` and `WEB_EXT_API_SECRET` for AMO unlisted signing.
4. Optionally set the repository variable `RUSTAB_CHROMIUM_BASE_URL` if you want a custom Pages or custom-domain URL. Otherwise the workflow defaults to `https://<owner>.github.io/<repo>/chromium`.
5. Push a tag like `vX.Y.Z`.

The workflow does not require a `gh-pages` branch. It deploys Pages directly from the workflow artifact, which keeps the repository history free of generated release files.

For normal push and pull request validation, `.github/workflows/ci.yml` runs formatting, clippy, tests, and `nix flake check` without needing signing secrets.

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
- **Chrome/Brave**: Go to `chrome://extensions` or `brave://extensions`, enable Developer Mode, and "Load unpacked" from `extensions/chrome/`
- **Orion**: Open `Tools > Extensions > Install from Disk` and choose `extensions/chrome/`
- **Firefox**: Open `extensions/firefox-signed/rustab@rustab.dev.xpi` in Firefox to install

`rustab install` uses the built-in Chromium extension ID by default. If you're testing a custom unpacked Chromium extension build with a different ID, pass `--chrome-extension-id <ID>`.

## Usage

```
rustab list                                # list all tabs (TSV)
rustab list --format json                  # list all tabs (JSON)
rustab list --browser brave                # list tabs from Brave only
rustab windows                             # list browser windows
rustab windows --format json               # list windows with scoped IDs and active tabs
rustab synced list --browser orion         # list synced Orion tabs cached locally on macOS
rustab synced list --browser orion --archived # inspect the newest non-empty archived Orion sync snapshot
rustab synced list --format json           # list synced tabs as JSON
rustab close b.18452.42 b.18452.99         # close specific tabs
rustab list | grep github | rustab close   # pipe pattern
rustab move --to-window b.18452.w.7 b.18452.42 # move a tab to a window
rustab list | grep YouTube | rustab move --to-window b.18452.w.7 # consolidate tabs
rustab move --to-tab b.18452.99 b.18452.42 # move a tab to the window containing another tab
rustab activate c.18452.42                 # focus a tab
rustab open https://example.com            # open URL in the first responsive browser
rustab open -b firefox https://x.com       # open in specific browser
rustab open --window b.18452.w.7 https://example.com # open in a specific window
rustab clients                             # show connected browsers, mediator PIDs, and sockets
```

`rustab synced list` is intentionally read-only. Today it supports Orion on macOS by reading Orion's locally cached sync state. By default it reads the live `browser_session_state.plist` view when available, falling back to Orion's current synced-tab plist on older layouts; `--archived` is a debugging escape hatch for the newest non-empty backup snapshot. Orion's live session-state data does not appear to include a friendly device name, so current entries may omit `device_id` even when archived snapshots have one.

## Development

The flake includes a dev shell with Rust toolchain and `web-ext` for Firefox extension signing:

```sh
nix run .#check-version-sync

# Refresh the checked-in signed Firefox XPI after extension changes
nix run .#refresh-firefox-xpi

# Re-run the consistency check before tagging a release
nix run .#check-version-sync
```

If the same Firefox version has already been submitted to AMO and is already public, `refresh-firefox-xpi` will download that existing signed XPI instead of failing on a duplicate-version error. That makes reruns and release recovery much calmer.

The Chromium release helper also works well from the dev shell:

```sh
nix develop -c package-chromium-release -- \
  --key /secure/path/rustab-chromium.pem \
  --base-url https://example.com/rustab/chromium
```

## License

AGPL-3.0-or-later
