{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { nixpkgs, self, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      packages.${system} = rec {
        rustab = pkgs.rustPlatform.buildRustPackage {
          pname = "rustab";
          version = "0.1.0";
          src = self;

          cargoHash = "sha256-LsQ7zggXlV8D3ueGEv9qkOFOE0YilYiJSET39vGc7xY=";

          postInstall = ''
            # Firefox native messaging host manifest
            install -Dm444 /dev/stdin \
              $out/lib/mozilla/native-messaging-hosts/rustab_mediator.json <<EOF
            {
              "name": "rustab_mediator",
              "description": "rustab native messaging host",
              "path": "$out/bin/rustab-mediator",
              "type": "stdio",
              "allowed_extensions": ["rustab@rustab.dev"]
            }
            EOF

            # Chromium native messaging host manifest
            install -Dm444 /dev/stdin \
              $out/etc/chromium/native-messaging-hosts/rustab_mediator.json <<EOF
            {
              "name": "rustab_mediator",
              "description": "rustab native messaging host",
              "path": "$out/bin/rustab-mediator",
              "type": "stdio",
              "allowed_origins": ["chrome-extension://nddbmnpippfilnjoebpcnfbpebnllbgo/"]
            }
            EOF
          '';

          meta = {
            description = "Browser tab management from the terminal";
            license = nixpkgs.lib.licenses.agpl3Plus;
            mainProgram = "rustab";
          };
        };

        chrome-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab-chrome-extension";
          version = "0.1.0";
          src = self;
          installPhase = ''
            mkdir -p $out
            cp extensions/chrome/* $out/
          '';
        };

        # AMO-signed XPI matching the rycee/nur firefox-addons layout
        # so it works with programs.firefox.profiles.<name>.extensions.packages
        # Re-sign after changes: source .web-ext-credentials && web-ext sign \
        #   --source-dir=extensions/firefox --channel=unlisted \
        #   --api-key=$WEB_EXT_API_KEY --api-secret=$WEB_EXT_API_SECRET
        firefox-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab";
          version = "0.1.0";
          src = self;

          passthru.addonId = "rustab@rustab.dev";

          installPhase = ''
            install -Dm444 extensions/firefox-signed/rustab@rustab.dev.xpi \
              "$out/share/mozilla/extensions/{ec8030f7-c20a-464f-9b0e-13a3a9e97384}/rustab@rustab.dev.xpi"
          '';
        };

        default = rustab;
      };

      devShells.${system}.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          gcc
          pkg-config
        ];
      };
    };
}
