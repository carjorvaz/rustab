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
    println!(
        "  1. Install the browser extension (load unpacked from extensions/chrome or open the signed Firefox XPI)"
    );
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
    if browser.is_firefox {
        build_firefox_manifest(mediator_path)
    } else {
        build_chrome_manifest(mediator_path, chrome_extension_id)
    }
}

fn manifest_target_dirs(home: &Path, browser: &BrowserManifestInfo) -> Vec<PathBuf> {
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

fn build_chrome_manifest(mediator_path: &Path, extension_id: &str) -> Result<String, String> {
    let mediator_path = manifest_path_string(mediator_path)?;
    serde_json::to_string_pretty(&json!({
        "name": NATIVE_HOST_NAME,
        "description": "rustab native messaging host",
        "path": mediator_path,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")]
    }))
    .map_err(|e| format!("failed to render Chromium manifest: {e}"))
}

fn build_firefox_manifest(mediator_path: &Path) -> Result<String, String> {
    let mediator_path = manifest_path_string(mediator_path)?;
    serde_json::to_string_pretty(&json!({
        "name": NATIVE_HOST_NAME,
        "description": "rustab native messaging host",
        "path": mediator_path,
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID]
    }))
    .map_err(|e| format!("failed to render Firefox manifest: {e}"))
}

fn manifest_path_string(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("manifest path is not valid UTF-8: {}", path.display()))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

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
