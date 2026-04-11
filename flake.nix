{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { nixpkgs, self, ... }:
    let
      packageChromiumReleaseFor = pkgs:
        pkgs.writeShellApplication {
          name = "package-chromium-release";
          runtimeInputs = [ pkgs.python3 ];
          text = ''
            exec python3 ${self}/scripts/package_chromium_release.py "$@"
          '';
        };
      appFor = pkgs: {
        type = "app";
        program = "${packageChromiumReleaseFor pkgs}/bin/package-chromium-release";
        meta = {
          description = "Package rustab's Chromium extension for managed distribution";
        };
      };
      version = "0.1.0";
      chromeExtensionId = "nddbmnpippfilnjoebpcnfbpebnllbgo";
      firefoxExtensionId = "rustab@rustab.dev";
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems
          (system: f system nixpkgs.legacyPackages.${system});
    in
    {
      lib = {
        inherit
          chromeExtensionId
          firefoxExtensionId
          version
          ;
        mkChromiumPolicy = {
          updateUrl,
          installationMode ? "force_installed",
          overrideUpdateUrl ? true,
          extensionId ? chromeExtensionId,
        }: {
          ExtensionSettings = {
            "${extensionId}" = {
              installation_mode = installationMode;
              update_url = updateUrl;
              override_update_url = overrideUpdateUrl;
            };
          };
        };
      };

      apps = forAllSystems (_: pkgs: {
        package-chromium-release = appFor pkgs;
      });

      packages = forAllSystems (system: pkgs: rec {
        package-chromium-release = packageChromiumReleaseFor pkgs;

        chrome-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab-chrome-extension";
          inherit version;
          src = self;
          installPhase = ''
            install -Dm444 extensions/chrome/manifest.json $out/manifest.json
            install -Dm444 extensions/chrome/background.js $out/background.js
            install -Dm444 extensions/chrome/icon48.png $out/icon48.png
            install -Dm444 extensions/chrome/icon128.png $out/icon128.png
          '';
          passthru = {
            extensionId = chromeExtensionId;
          };
        };

        # AMO-signed XPI matching the rycee/nur firefox-addons layout
        # so it works with programs.firefox.profiles.<name>.extensions.packages
        # Re-sign after changes: source .web-ext-credentials && web-ext sign \
        #   --source-dir=extensions/firefox --channel=unlisted \
        #   --api-key=$WEB_EXT_API_KEY --api-secret=$WEB_EXT_API_SECRET
        firefox-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab";
          inherit version;
          src = self;

          passthru.addonId = firefoxExtensionId;

          installPhase = ''
            install -Dm444 extensions/firefox-signed/${firefoxExtensionId}.xpi \
              "$out/share/mozilla/extensions/{ec8030f7-c20a-464f-9b0e-13a3a9e97384}/${firefoxExtensionId}.xpi"
          '';
        };

        rustab = pkgs.rustPlatform.buildRustPackage {
          pname = "rustab";
          inherit version;
          src = self;

          cargoHash = "sha256-LsQ7zggXlV8D3ueGEv9qkOFOE0YilYiJSET39vGc7xY=";

          postInstall = ''
            # Firefox native messaging host manifest
            mkdir -p $out/lib/mozilla/native-messaging-hosts
            cat > $out/lib/mozilla/native-messaging-hosts/rustab_mediator.json <<EOF
            {
              "name": "rustab_mediator",
              "description": "rustab native messaging host",
              "path": "$out/bin/rustab-mediator",
              "type": "stdio",
              "allowed_extensions": ["${firefoxExtensionId}"]
            }
            EOF
            chmod 444 $out/lib/mozilla/native-messaging-hosts/rustab_mediator.json

            # Chromium native messaging host manifest
            mkdir -p $out/etc/chromium/native-messaging-hosts
            cat > $out/etc/chromium/native-messaging-hosts/rustab_mediator.json <<EOF
            {
              "name": "rustab_mediator",
              "description": "rustab native messaging host",
              "path": "$out/bin/rustab-mediator",
              "type": "stdio",
              "allowed_origins": ["chrome-extension://${chromeExtensionId}/"]
            }
            EOF
            chmod 444 $out/etc/chromium/native-messaging-hosts/rustab_mediator.json
          '';

          passthru = {
            inherit
              chromeExtensionId
              firefoxExtensionId
              version
              ;
            chromeExtension = chrome-extension;
            firefoxExtension = firefox-extension;
          };

          meta = {
            description = "Browser tab management from the terminal";
            license = nixpkgs.lib.licenses.agpl3Plus;
            mainProgram = "rustab";
            platforms = nixpkgs.lib.platforms.unix;
          };
        };

        default = rustab;
      });

      checks = forAllSystems (system: pkgs: {
        package-chromium-release = self.packages.${system}.package-chromium-release;
        chrome-extension = self.packages.${system}.chrome-extension;
        firefox-extension = self.packages.${system}.firefox-extension;
        rustab = self.packages.${system}.rustab;
      });

      devShells = forAllSystems (system: pkgs: {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            pkg-config
            python3
            web-ext
          ];
        };
      });
    };
}
