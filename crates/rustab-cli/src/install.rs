use rustab_protocol::{
    BrowserManifestInfo, BROWSERS, CHROME_EXTENSION_ID, FIREFOX_EXTENSION_ID, NATIVE_HOST_NAME,
};
use serde_json::json;
use std::path::{Path, PathBuf};

pub fn cmd_install(mediator_path: Option<PathBuf>, chrome_extension_id: Option<String>) -> i32 {
    let mediator = match mediator_path
        .or_else(find_sibling_mediator)
        .or_else(|| find_in_path("rustab-mediator"))
    {
        Some(path) => path,
        None => {
            eprintln!("Could not find rustab-mediator. Use --mediator-path to specify.");
            return 1;
        }
    };

    let mediator_abs = match std::fs::canonicalize(&mediator) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Cannot resolve mediator path {}: {e}", mediator.display());
            return 1;
        }
    };

    let home = match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home),
        None => {
            eprintln!("$HOME not set");
            return 1;
        }
    };

    let using_default_chrome_extension_id = chrome_extension_id.is_none();
    let chrome_ext_id = chrome_extension_id.unwrap_or_else(|| CHROME_EXTENSION_ID.to_string());

    let mut installed_locations = 0;
    let mut installed_browsers = 0;
    let mut installed_orion = false;
    let mut installed_non_orion = false;

    for browser in BROWSERS {
        let config_path = home.join(browser.config_dir);
        if !config_path.exists() {
            continue;
        }

        let manifest = match build_manifest(browser, &mediator_abs, &chrome_ext_id) {
            Ok(manifest) => manifest,
            Err(error) => {
                eprintln!("{}: {error}", browser.name);
                continue;
            }
        };

        let mut wrote_manifest_for_browser = false;

        for manifest_dir in manifest_target_dirs(&home, browser) {
            if let Err(e) = std::fs::create_dir_all(&manifest_dir) {
                eprintln!(
                    "{}: failed to create manifest dir {}: {e}",
                    browser.name,
                    manifest_dir.display()
                );
                continue;
            }

            let manifest_path = manifest_dir.join(format!("{NATIVE_HOST_NAME}.json"));
            match std::fs::write(&manifest_path, &manifest) {
                Ok(()) => {
                    println!("{}: installed {}", browser.name, manifest_path.display());
                    wrote_manifest_for_browser = true;
                    installed_locations += 1;
                }
                Err(e) => eprintln!("{}: failed to write manifest: {e}", browser.name),
            }
        }

        if wrote_manifest_for_browser {
            installed_browsers += 1;
            if browser.name == "orion" {
                installed_orion = true;
            } else {
                installed_non_orion = true;
            }
        }
    }

    if installed_locations == 0 {
        eprintln!("No browsers detected. Check that browser config directories exist.");
        return 1;
    }

    println!(
        "\nInstalled manifests at {installed_locations} location(s) across {installed_browsers} browser(s)."
    );
    if using_default_chrome_extension_id {
        println!("Using built-in Chromium extension ID: {CHROME_EXTENSION_ID}");
        println!("Pass --chrome-extension-id to override it for a custom unpacked build.");
    }
    println!("Next steps:");
    if installed_orion && installed_non_orion {
        println!("  1. Install the browser extension:");
        println!("     - Chrome/Chromium-family: load unpacked from extensions/chrome");
        println!("     - Firefox: open the signed Firefox XPI");
        println!(
            "     - Orion: load unpacked from extensions/orion/ or use the flake orion-extension package"
        );
    } else if installed_orion {
        println!(
            "  1. Install the Orion extension (load unpacked from extensions/orion/ or use the flake orion-extension package)"
        );
    } else {
        println!(
            "  1. Install the browser extension (load unpacked from extensions/chrome or open the signed Firefox XPI)"
        );
    }
    println!("  2. Restart your browser");
    println!("  3. Run `rustab clients` to verify the connection");

    0
}

fn find_sibling_mediator() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let sibling = exe.with_file_name("rustab-mediator");
    sibling.is_file().then_some(sibling)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

fn build_manifest(
    browser: &BrowserManifestInfo,
    mediator_path: &Path,
    chrome_extension_id: &str,
) -> Result<String, String> {
    let mediator_path = manifest_path_string(mediator_path)?;
    let (allowlist_key, allowed_extension) =
        manifest_extension_allowlist(browser, chrome_extension_id);
    let mut manifest = json!({
        "name": NATIVE_HOST_NAME,
        "description": "rustab native messaging host",
        "path": mediator_path,
        "type": "stdio",
    });
    manifest[allowlist_key] = json!([allowed_extension]);

    serde_json::to_string_pretty(&manifest).map_err(|e| {
        format!(
            "failed to render {} manifest: {e}",
            if browser.is_firefox {
                "Firefox"
            } else {
                "Chromium"
            }
        )
    })
}

pub(crate) fn manifest_extension_allowlist(
    browser: &BrowserManifestInfo,
    chrome_extension_id: &str,
) -> (&'static str, String) {
    if browser.is_firefox {
        ("allowed_extensions", FIREFOX_EXTENSION_ID.to_string())
    } else {
        (
            "allowed_origins",
            format!("chrome-extension://{chrome_extension_id}/"),
        )
    }
}

pub(crate) fn manifest_target_dirs(home: &Path, browser: &BrowserManifestInfo) -> Vec<PathBuf> {
    let mut dirs = vec![home.join(browser.config_dir).join(browser.manifest_subdir)];

    #[cfg(target_os = "macos")]
    if browser.name == "brave" && !browser.is_firefox {
        // Brave on macOS does not reliably discover per-profile native
        // messaging hosts from its branded application-support directory.
        // Install fallback copies in the standard Chromium-family user paths
        // so sideloaded Rustab works regardless of which lookup variant Brave
        // uses on a given release.
        dirs.push(home.join("Library/Application Support/Chromium/NativeMessagingHosts"));
        dirs.push(home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts"));
    }

    dirs.sort();
    dirs.dedup();
    dirs
}

fn manifest_path_string(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("manifest path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_manifest_uses_custom_extension_origin() {
        let chromium = BROWSERS
            .iter()
            .find(|browser| !browser.is_firefox)
            .expect("chromium browser metadata");
        let manifest = serde_json::from_str::<serde_json::Value>(
            &build_manifest(
                chromium,
                Path::new("/usr/local/bin/rustab-mediator"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("build manifest"),
        )
        .expect("parse manifest");

        assert_eq!(
            manifest,
            json!({
                "name": NATIVE_HOST_NAME,
                "description": "rustab native messaging host",
                "path": "/usr/local/bin/rustab-mediator",
                "type": "stdio",
                "allowed_origins": ["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"]
            })
        );
    }

    #[test]
    fn firefox_manifest_uses_firefox_extension_allowlist() {
        let firefox = BROWSERS
            .iter()
            .find(|browser| browser.is_firefox)
            .expect("firefox browser metadata");
        let manifest = serde_json::from_str::<serde_json::Value>(
            &build_manifest(
                firefox,
                Path::new("/usr/local/bin/rustab-mediator"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("build manifest"),
        )
        .expect("parse manifest");

        assert_eq!(
            manifest,
            json!({
                "name": NATIVE_HOST_NAME,
                "description": "rustab native messaging host",
                "path": "/usr/local/bin/rustab-mediator",
                "type": "stdio",
                "allowed_extensions": [FIREFOX_EXTENSION_ID]
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn brave_mac_manifest_dirs_include_fallbacks() {
        let brave = BROWSERS
            .iter()
            .find(|browser| browser.name == "brave")
            .expect("brave browser metadata");

        let dirs = manifest_target_dirs(Path::new("/Users/test"), brave);

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/Users/test/Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts"),
                PathBuf::from("/Users/test/Library/Application Support/Chromium/NativeMessagingHosts"),
                PathBuf::from("/Users/test/Library/Application Support/Google/Chrome/NativeMessagingHosts"),
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn orion_mac_manifest_dir_uses_orion_application_support_path() {
        let orion = BROWSERS
            .iter()
            .find(|browser| browser.name == "orion")
            .expect("orion browser metadata");

        let dirs = manifest_target_dirs(Path::new("/Users/test"), orion);

        assert_eq!(
            dirs,
            vec![PathBuf::from(
                "/Users/test/Library/Application Support/Orion/NativeMessagingHosts"
            ),]
        );
    }
}
