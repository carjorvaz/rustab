use crate::client::{discover_sockets, send_rpc, BrowserSocket};
use crate::install::manifest_target_dirs;
use rustab_protocol::{
    browser_prefix, socket_dir, validate_socket_dir, BrowserManifestInfo, RpcRequest, TabInfo,
    BROWSERS, CHROME_EXTENSION_ID, FIREFOX_EXTENSION_ID, LIST_TABS_METHOD, LIST_WINDOWS_METHOD,
    NATIVE_HOST_NAME,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Report {
    ok: usize,
    warnings: usize,
    errors: usize,
}

impl Report {
    fn ok(&mut self, message: impl AsRef<str>) {
        self.ok += 1;
        println!("ok: {}", message.as_ref());
    }

    fn warn(&mut self, message: impl AsRef<str>) {
        self.warnings += 1;
        println!("warn: {}", message.as_ref());
    }

    fn error(&mut self, message: impl AsRef<str>) {
        self.errors += 1;
        println!("error: {}", message.as_ref());
    }

    fn exit_code(&self) -> i32 {
        i32::from(self.errors > 0)
    }
}

pub async fn cmd_doctor(browser_filter: Option<&str>, chrome_extension_id: Option<&str>) -> i32 {
    let mut report = Report::default();
    let chrome_extension_id = chrome_extension_id.unwrap_or(CHROME_EXTENSION_ID);

    check_socket_dir(&mut report);
    check_native_manifests(&mut report, browser_filter, chrome_extension_id);
    check_connected_browsers(&mut report, browser_filter).await;

    println!(
        "summary: {} ok, {} warning(s), {} error(s)",
        report.ok, report.warnings, report.errors
    );

    report.exit_code()
}

fn check_socket_dir(report: &mut Report) {
    let dir = socket_dir();
    if !dir.exists() {
        report.warn(format!(
            "socket directory does not exist yet: {}",
            dir.display()
        ));
        return;
    }

    match validate_socket_dir(&dir) {
        Ok(()) => report.ok(format!("socket directory is private: {}", dir.display())),
        Err(error) => report.error(format!("socket directory {}: {error}", dir.display())),
    }
}

fn check_native_manifests(
    report: &mut Report,
    browser_filter: Option<&str>,
    chrome_extension_id: &str,
) {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        report.warn("$HOME is not set; skipping native messaging manifest checks");
        return;
    };

    let current_mediator = current_mediator_path();
    let mut detected_browser_configs = 0;

    for browser in BROWSERS {
        if let Some(filter) = browser_filter {
            if browser.name != filter {
                continue;
            }
        }

        let config_path = home.join(browser.config_dir);
        if !config_path.exists() {
            continue;
        }
        detected_browser_configs += 1;

        let manifest_paths = manifest_target_dirs(&home, browser)
            .into_iter()
            .map(|dir| dir.join(format!("{NATIVE_HOST_NAME}.json")))
            .collect::<Vec<_>>();

        let mut present_manifests = 0;
        for manifest_path in manifest_paths {
            if !manifest_path.exists() {
                report.warn(format!(
                    "{} native host manifest is missing: {}",
                    browser.name,
                    manifest_path.display()
                ));
                continue;
            }

            present_manifests += 1;
            match validate_manifest(
                &manifest_path,
                browser,
                current_mediator.as_deref(),
                chrome_extension_id,
            ) {
                Ok(warnings) => {
                    report.ok(format!(
                        "{} native host manifest is readable: {}",
                        browser.name,
                        manifest_path.display()
                    ));
                    for warning in warnings {
                        report.warn(warning);
                    }
                }
                Err(errors) => {
                    for error in errors {
                        report.error(error);
                    }
                }
            }
        }

        if present_manifests == 0 {
            report.error(format!(
                "{} profile exists, but no rustab native host manifests were found",
                browser.name
            ));
        }

        #[cfg(target_os = "macos")]
        if browser.name == "orion" {
            check_orion_extension_install(report, &home, chrome_extension_id);
        }
    }

    if detected_browser_configs == 0 {
        match browser_filter {
            Some(browser) => report.warn(format!("no local config directory found for {browser}")),
            None => report.warn("no known local browser config directories found"),
        }
    }
}

#[cfg(target_os = "macos")]
fn check_orion_extension_install(report: &mut Report, home: &Path, chrome_extension_id: &str) {
    let manifest_path = home
        .join("Library/Application Support/Orion/Defaults/Extensions")
        .join(chrome_extension_id)
        .join("manifest.json");

    if !manifest_path.exists() {
        report.warn(format!(
            "orion extension is not installed at {}; load the unpacked extension from extensions/orion/",
            manifest_path.display()
        ));
        return;
    }

    let value = match std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read: {error}"))
        .and_then(|contents| {
            serde_json::from_str::<Value>(&contents)
                .map_err(|error| format!("invalid JSON: {error}"))
        }) {
        Ok(value) => value,
        Err(error) => {
            report.error(format!("{}: {error}", manifest_path.display()));
            return;
        }
    };

    let manifest_version = value.get("manifest_version").and_then(Value::as_u64);
    let uses_service_worker = value
        .get("background")
        .and_then(|background| background.get("service_worker"))
        .is_some();

    match (manifest_version, uses_service_worker) {
        (Some(2), _) => report.ok(format!(
            "orion extension is the Manifest V2 background-page build: {}",
            manifest_path.display()
        )),
        (Some(3), true) => report.error(format!(
            "orion extension is the Chromium Manifest V3 service-worker build: {}; install the Orion build from extensions/orion/ or the flake's orion-extension package, then reload Orion",
            manifest_path.display()
        )),
        (Some(version), _) => report.warn(format!(
            "orion extension has unexpected manifest_version {version}: {}",
            manifest_path.display()
        )),
        (None, _) => report.warn(format!(
            "orion extension manifest has no numeric manifest_version: {}",
            manifest_path.display()
        )),
    }
}

async fn check_connected_browsers(report: &mut Report, browser_filter: Option<&str>) {
    let sockets = discover_sockets(browser_filter);
    if sockets.is_empty() {
        match browser_filter {
            Some(browser) => report.error(format!("no connected browser mediators for {browser}")),
            None => report.error("no connected browser mediators"),
        }
        return;
    }

    report.ok(format!("{} connected browser mediator(s)", sockets.len()));

    for socket in sockets {
        check_browser_socket(report, &socket).await;
    }
}

/// Send a window-list RPC and return the window count, or the error string.
async fn window_check_count(socket: &BrowserSocket) -> Result<usize, String> {
    send_rpc::<Vec<Value>>(socket, &RpcRequest::new(LIST_WINDOWS_METHOD, json!({})))
        .await
        .map(|w| w.len())
}

async fn check_browser_socket(report: &mut Report, socket: &BrowserSocket) {
    let label = format!(
        "{} ({}, pid {}, prefix {})",
        socket.browser,
        socket.path.display(),
        socket.pid,
        browser_prefix(&socket.browser)
    );

    match send_rpc::<Vec<TabInfo>>(socket, &RpcRequest::new(LIST_TABS_METHOD, json!({}))).await {
        Ok(tabs) => report.ok(format!("{label}: list_tabs returned {} tab(s)", tabs.len())),
        Err(error) => report.error(format!("{label}: list_tabs failed: {error}")),
    }

    match window_check_count(socket).await {
        Ok(count) => report.ok(format!("{label}: list_windows returned {count} window(s)")),
        Err(error) if error.contains("unknown method") => report.error(format!(
            "{label}: list_windows is unsupported; update or reload the rustab browser extension"
        )),
        Err(error) => report.error(format!("{label}: list_windows failed: {error}")),
    }
}

fn validate_manifest(
    manifest_path: &Path,
    browser: &BrowserManifestInfo,
    current_mediator: Option<&Path>,
    chrome_extension_id: &str,
) -> Result<Vec<String>, Vec<String>> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    let value = match std::fs::read_to_string(manifest_path)
        .map_err(|error| format!("{}: failed to read: {error}", manifest_path.display()))
        .and_then(|contents| {
            serde_json::from_str::<Value>(&contents)
                .map_err(|error| format!("{}: invalid JSON: {error}", manifest_path.display()))
        }) {
        Ok(value) => value,
        Err(error) => return Err(vec![error]),
    };

    if value.get("name").and_then(Value::as_str) != Some(NATIVE_HOST_NAME) {
        errors.push(format!(
            "{}: native host name is not {NATIVE_HOST_NAME:?}",
            manifest_path.display()
        ));
    }

    let manifest_mediator = value.get("path").and_then(Value::as_str).map(PathBuf::from);
    match manifest_mediator.as_deref() {
        Some(path) if path.is_file() => {
            if let Some(current_mediator) = current_mediator {
                match (canonicalize(path), canonicalize(current_mediator)) {
                    (Some(manifest_mediator), Some(current_mediator))
                        if manifest_mediator != current_mediator =>
                    {
                        warnings.push(format!(
                            "{}: points to {}, but the current rustab-mediator sibling is {}",
                            manifest_path.display(),
                            manifest_mediator.display(),
                            current_mediator.display()
                        ));
                    }
                    _ => {}
                }
            }
        }
        Some(path) => errors.push(format!(
            "{}: mediator path does not exist: {}",
            manifest_path.display(),
            path.display()
        )),
        None => errors.push(format!(
            "{}: native host manifest has no string path",
            manifest_path.display()
        )),
    }

    if browser.is_firefox {
        if !json_array_contains(&value, "allowed_extensions", FIREFOX_EXTENSION_ID) {
            errors.push(format!(
                "{}: allowed_extensions does not include {FIREFOX_EXTENSION_ID}",
                manifest_path.display()
            ));
        }
    } else {
        let allowed_origin = format!("chrome-extension://{chrome_extension_id}/");
        if !json_array_contains(&value, "allowed_origins", &allowed_origin) {
            errors.push(format!(
                "{}: allowed_origins does not include {allowed_origin}",
                manifest_path.display()
            ));
        }
    }

    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(errors)
    }
}

fn json_array_contains(value: &Value, key: &str, expected: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(expected)))
}

fn current_mediator_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|path| path.with_file_name("rustab-mediator"))
        .filter(|path| path.is_file())
}

fn canonicalize(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_array_contains_expected_string() {
        let value = json!({
            "allowed_origins": [
                "chrome-extension://nddbmnpippfilnjoebpcnfbpebnllbgo/"
            ]
        });

        assert!(json_array_contains(
            &value,
            "allowed_origins",
            "chrome-extension://nddbmnpippfilnjoebpcnfbpebnllbgo/"
        ));
        assert!(!json_array_contains(&value, "allowed_origins", "other"));
        assert!(!json_array_contains(&value, "missing", "other"));
    }

    #[test]
    fn validate_manifest_accepts_custom_chrome_extension_id() {
        let custom_extension_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let browser = BROWSERS
            .iter()
            .find(|browser| !browser.is_firefox)
            .expect("chromium browser metadata");
        let mediator = std::env::current_exe().expect("current test executable");
        let manifest_path = std::env::temp_dir().join(format!(
            "rustab-doctor-manifest-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos()
        ));
        std::fs::write(
            &manifest_path,
            serde_json::to_string(&json!({
                "name": NATIVE_HOST_NAME,
                "path": mediator,
                "type": "stdio",
                "allowed_origins": [format!("chrome-extension://{custom_extension_id}/")]
            }))
            .expect("render manifest"),
        )
        .expect("write manifest");

        assert!(validate_manifest(&manifest_path, browser, None, custom_extension_id).is_ok());

        let errors = validate_manifest(&manifest_path, browser, None, CHROME_EXTENSION_ID)
            .expect_err("default expected ID must stay strict");
        assert!(errors
            .iter()
            .any(|error| error.contains(&format!("chrome-extension://{CHROME_EXTENSION_ID}/"))));

        std::fs::remove_file(manifest_path).expect("remove manifest");
    }
}
