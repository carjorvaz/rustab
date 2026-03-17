# rustab

Browser tab management from the terminal. A Rust replacement for [brotab](https://github.com/balta2ar/brotab).

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
- NixOS/flake-native packaging

## Architecture

```
Browser extension  <--native messaging (stdio)-->  rustab-mediator  <--Unix socket-->  rustab CLI
```

Each browser instance gets its own mediator process and Unix socket at `/tmp/rustab-{user}/{browser}-{pid}.sock`. The CLI discovers mediators by scanning this directory and filtering out stale sockets (dead PIDs).

Tab IDs are prefixed by browser: `c.123` (Chrome), `b.456` (Brave), `f.789` (Firefox), etc.

## Installation

### NixOS (flake)

Add rustab as a flake input:

```nix
{
  inputs.rustab.url = "github:cajorvaz/rustab";
  inputs.rustab.inputs.nixpkgs.follows = "nixpkgs";
}
```

The flake provides three packages:
- `rustab` -- CLI + mediator binaries with native messaging manifests
- `chrome-extension` -- unpacked Chrome extension directory
- `firefox-extension` -- AMO-signed XPI for Firefox

#### Brave / Chrome / Chromium

Load the extension via `--load-extension`:

```nix
# In your browser overlay or wrapper
"--load-extension=${inputs.rustab.packages.${system}.chrome-extension}"
```

Add the native messaging host manifest:

```nix
home.file.".config/BraveSoftware/Brave-Browser/NativeMessagingHosts/rustab_mediator.json".source =
  "${inputs.rustab.packages.${system}.rustab}/etc/chromium/native-messaging-hosts/rustab_mediator.json";
```

#### Firefox

```nix
# home-manager
programs.firefox = {
  nativeMessagingHosts = [ pkgs.rustab ];
  profiles.default.extensions.packages = [ pkgs.rustab-firefox-extension ];
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

## License

AGPL-3.0-or-later
