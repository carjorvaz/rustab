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

          cargoHash = "sha256-2ouSy1W48Xmi7zqiLorDkXKaP6YoDgSCgig24VjS7K0=";

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
            license = nixpkgs.lib.licenses.mit;
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

        # Packaged as an XPI matching the rycee/nur firefox-addons layout
        # so it works with programs.firefox.profiles.<name>.extensions.packages
        firefox-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab";
          version = "0.1.0";
          src = self;

          addonId = "rustab@rustab.dev";

          buildPhase = ''
            cd extensions/firefox
            ${pkgs.zip}/bin/zip -r $TMPDIR/rustab.xpi ./*
          '';

          installPhase = ''
            install -Dm444 $TMPDIR/rustab.xpi \
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
