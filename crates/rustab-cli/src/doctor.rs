use crate::client::{discover_sockets, send_rpc, BrowserSocket};
use crate::install::manifest_target_dirs;
use rustab_protocol::{
    browser_prefix, socket_dir, validate_socket_dir, BrowserManifestInfo, RpcRequest, TabInfo,
    WindowInfo, BROWSERS, CHROME_EXTENSION_ID, FIREFOX_EXTENSION_ID, LIST_TABS_METHOD,
    LIST_WINDOWS_METHOD, NATIVE_HOST_NAME,
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

pub async fn cmd_doctor(browser_filter: Option<&str>) -> i32 {
    let mut report = Report::default();

    check_socket_dir(&mut report);
    check_native_manifests(&mut report, browser_filter);
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

fn check_native_manifests(report: &mut Report, browser_filter: Option<&str>) {
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
            match validate_manifest(&manifest_path, browser, current_mediator.as_deref()) {
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
    }

    if detected_browser_configs == 0 {
        match browser_filter {
            Some(browser) => report.warn(format!("no local config directory found for {browser}")),
            None => report.warn("no known local browser config directories found"),
        }
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

    match send_rpc::<Vec<WindowInfo>>(socket, &RpcRequest::new(LIST_WINDOWS_METHOD, json!({})))
        .await
    {
        Ok(windows) => report.ok(format!(
            "{label}: list_windows returned {} window(s)",
            windows.len()
        )),
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
        let allowed_origin = format!("chrome-extension://{CHROME_EXTENSION_ID}/");
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
}
