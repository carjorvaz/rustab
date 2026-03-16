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

          useFetchCargoVendor = true;
          cargoHash = nixpkgs.lib.fakeHash;

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
              "allowed_origins": ["chrome-extension://PLACEHOLDER_EXTENSION_ID/"]
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

        firefox-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab-firefox-extension";
          version = "0.1.0";
          src = self;
          installPhase = ''
            mkdir -p $out
            cp extensions/firefox/* $out/
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
