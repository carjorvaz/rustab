{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { nixpkgs, self, ... }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      chromeManifest = builtins.fromJSON (builtins.readFile ./extensions/chrome/manifest.json);
      firefoxManifest = builtins.fromJSON (builtins.readFile ./extensions/firefox/manifest.json);
      packageChromiumReleaseFor = pkgs:
        pkgs.writeShellApplication {
          name = "package-chromium-release";
          runtimeInputs = [ pkgs.python3 ];
          text = ''
            exec python3 ${self}/scripts/package_chromium_release.py "$@"
          '';
        };
      refreshFirefoxXpiFor = pkgs:
        pkgs.writeShellApplication {
          name = "refresh-firefox-xpi";
          runtimeInputs = [
            pkgs.bash
            pkgs.coreutils
            pkgs.python3
            pkgs.web-ext
          ];
          text = ''
            exec ${pkgs.bash}/bin/bash ${self}/scripts/refresh_firefox_xpi.sh "$@"
          '';
        };
      checkVersionSyncFor = pkgs:
        pkgs.writeShellApplication {
          name = "check-version-sync";
          runtimeInputs = [ pkgs.python3 ];
          text = ''
            exec python3 ${self}/scripts/check_versions.py "$@"
          '';
        };
      appFor = pkgs: {
        type = "app";
        program = "${packageChromiumReleaseFor pkgs}/bin/package-chromium-release";
        meta = {
          description = "Package rustab's Chromium extension for managed distribution";
        };
      };
      version =
        let
          cargoVersion = cargoToml.workspace.package.version;
        in
        assert chromeManifest.version == cargoVersion;
        assert firefoxManifest.version == cargoVersion;
        cargoVersion;
      chromeExtensionId = "nddbmnpippfilnjoebpcnfbpebnllbgo";
      firefoxExtensionId = firefoxManifest.browser_specific_settings.gecko.id;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      extensionJavascriptFiles = [
        "extensions/shared/background_core.js"
        "extensions/chrome/background.js"
        "extensions/chrome/background_core.js"
        "extensions/firefox/background.js"
        "extensions/firefox/background_core.js"
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
        refresh-firefox-xpi = {
          type = "app";
          program = "${refreshFirefoxXpiFor pkgs}/bin/refresh-firefox-xpi";
          meta = {
            description = "Refresh Rustab's checked-in signed Firefox XPI";
          };
        };
        check-version-sync = {
          type = "app";
          program = "${checkVersionSyncFor pkgs}/bin/check-version-sync";
          meta = {
            description = "Verify Rustab's version stays in sync across release artifacts";
          };
        };
      });

      packages = forAllSystems (system: pkgs: rec {
        package-chromium-release = packageChromiumReleaseFor pkgs;
        refresh-firefox-xpi = refreshFirefoxXpiFor pkgs;
        check-version-sync = checkVersionSyncFor pkgs;

        chrome-extension = pkgs.stdenvNoCC.mkDerivation {
          pname = "rustab-chrome-extension";
          inherit version;
          src = self;
          installPhase = ''
            install -Dm444 extensions/chrome/manifest.json $out/manifest.json
            install -Dm444 extensions/chrome/background.js $out/background.js
            install -Dm444 extensions/chrome/background_core.js $out/background_core.js
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

          cargoHash = "sha256-DeipIR/JRCl5nuLm8ic/f6c98uSx7oLxPse4PHY7PVM=";

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
        version-sync = pkgs.runCommand "rustab-version-sync" {
          nativeBuildInputs = [ self.packages.${system}.check-version-sync ];
        } ''
          check-version-sync
          touch $out
        '';
        refresh-firefox-xpi = self.packages.${system}.refresh-firefox-xpi;
        package-chromium-release = self.packages.${system}.package-chromium-release;
        chrome-extension = self.packages.${system}.chrome-extension;
        firefox-extension = self.packages.${system}.firefox-extension;
        extension-js-syntax = pkgs.runCommand "rustab-extension-js-syntax" {
          nativeBuildInputs = [ pkgs.nodejs ];
        } ''
          for file in ${nixpkgs.lib.escapeShellArgs extensionJavascriptFiles}; do
            node --check "${self}/$file"
          done
          touch $out
        '';
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
            nodejs
            web-ext
          ];
        };
      });
    };
}
