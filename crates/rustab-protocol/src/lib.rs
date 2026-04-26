use std::io;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const REQUEST_TIMEOUT_SECS: u64 = 10;
pub const LIST_TABS_METHOD: &str = "list_tabs";
pub const LIST_WINDOWS_METHOD: &str = "list_windows";
pub const CLOSE_TABS_METHOD: &str = "close_tabs";
pub const ACTIVATE_TAB_METHOD: &str = "activate_tab";
pub const OPEN_TAB_METHOD: &str = "open_tab";
pub const MOVE_TABS_METHOD: &str = "move_tabs";

/// Read a native-messaging-framed JSON message.
///
/// Wire format: 4-byte little-endian u32 length prefix, then UTF-8 JSON payload.
/// This is the same framing used by Chrome/Firefox native messaging over stdio
/// and by our Unix socket protocol.
pub async fn read_message<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<serde_json::Value> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "empty message",
        ));
    }
    if len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message exceeds 64 MiB",
        ));
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;

    serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write a native-messaging-framed JSON message.
pub async fn write_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &serde_json::Value,
) -> std::io::Result<()> {
    let payload = serde_json::to_vec(msg)?;

    if payload.len() > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message exceeds 1 MiB outbound limit",
        ));
    }

    let len = (payload.len() as u32).to_le_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Request envelope shared by the CLI, mediator, and browser extension.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl RpcRequest {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            id: 1,
            method: method.into(),
            params,
        }
    }
}

/// Response envelope for browser-extension RPC replies.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RpcResponse<T = serde_json::Value> {
    pub id: u64,
    #[serde(default = "missing_result")]
    pub result: Option<T>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

impl<T> RpcResponse<T> {
    pub fn into_result(self) -> Result<T, String> {
        if let Some(error) = self.error {
            return Err(json_error_message(&error));
        }

        self.result.ok_or_else(|| "invalid response".to_string())
    }
}

fn json_error_message(error: &serde_json::Value) -> String {
    error
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn missing_result<T>() -> Option<T> {
    None
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct TabInfo {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub active: bool,
    pub window_id: u64,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    #[serde(default)]
    pub focused: bool,
    #[serde(default, rename = "type")]
    pub window_type: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub incognito: bool,
    #[serde(default)]
    pub tab_count: u64,
    #[serde(default)]
    pub active_tab_id: Option<u64>,
    #[serde(default)]
    pub active_tab_title: String,
    #[serde(default)]
    pub active_tab_url: String,
}

/// Socket directory: `/tmp/rustab-{uid}/`
pub fn socket_dir() -> PathBuf {
    #[cfg(unix)]
    {
        let uid = unsafe { geteuid() };
        PathBuf::from(format!("/tmp/rustab-{uid}"))
    }

    #[cfg(not(unix))]
    {
        let username = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
        PathBuf::from(format!("/tmp/rustab-{username}"))
    }
}

/// Create and validate the per-user socket directory before binding sockets.
pub fn prepare_socket_dir() -> io::Result<PathBuf> {
    let dir = socket_dir();

    #[cfg(unix)]
    {
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }

        use std::os::unix::fs::PermissionsExt;
        let metadata = trusted_socket_dir_metadata(&dir)?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o700 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        validate_socket_dir(&dir)?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&dir)?;
    }

    Ok(dir)
}

/// Ensure an existing socket directory is owned by the current user and private.
pub fn validate_socket_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = trusted_socket_dir_metadata(dir)?;

        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("socket directory permissions are {mode:o}, expected 700"),
            ));
        }
    }

    #[cfg(not(unix))]
    {
        let metadata = std::fs::metadata(dir)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "socket path is not a directory",
            ));
        }
    }

    Ok(())
}

#[cfg(unix)]
fn trusted_socket_dir_metadata(dir: &Path) -> io::Result<std::fs::Metadata> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(dir)?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket path is not a directory",
        ));
    }

    let uid = unsafe { geteuid() };
    if metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "socket directory is owned by uid {}, expected uid {}",
                metadata.uid(),
                uid
            ),
        ));
    }

    Ok(metadata)
}

/// Socket path for a given browser and PID: `/tmp/rustab-{user}/{browser}-{pid}.sock`
pub fn socket_path(browser: &str, pid: u32) -> PathBuf {
    socket_dir().join(format!("{browser}-{pid}.sock"))
}

/// Parse a socket filename into (browser, pid).
pub fn parse_socket_name(filename: &str) -> Option<(String, u32)> {
    let stem = filename.strip_suffix(".sock")?;
    let (browser, pid_str) = stem.rsplit_once('-')?;
    let pid = pid_str.parse().ok()?;
    Some((browser.to_string(), pid))
}

/// Short prefix for a browser name, used in tab ID formatting.
pub fn browser_prefix(browser: &str) -> &str {
    match browser {
        "firefox" => "f",
        "chrome" => "c",
        "brave" => "b",
        "orion" => "or",
        "chromium" => "cr",
        "zen" => "z",
        "edge" => "e",
        "vivaldi" => "v",
        _ => "u",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabRef<'a> {
    pub prefix: &'a str,
    pub mediator_pid: Option<u32>,
    pub tab_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRef<'a> {
    pub prefix: &'a str,
    pub mediator_pid: Option<u32>,
    pub window_id: u64,
}

/// Format a tab ID using the browser prefix, mediator PID, and browser tab ID.
pub fn format_tab_id(prefix: &str, mediator_pid: u32, tab_id: u64) -> String {
    format!("{prefix}.{mediator_pid}.{tab_id}")
}

/// Format a window ID using the browser prefix, mediator PID, and browser window ID.
pub fn format_window_id(prefix: &str, mediator_pid: u32, window_id: u64) -> String {
    format!("{prefix}.{mediator_pid}.w.{window_id}")
}

/// Parse a tab ID.
///
/// Accepts either the current `prefix.pid.tab_id` format or the legacy
/// `prefix.tab_id` shorthand for single-mediator setups.
pub fn parse_tab_id(s: &str) -> Option<TabRef<'_>> {
    let mut parts = s.split('.');
    let prefix = parts.next()?;
    let second = parts.next()?;
    let third = parts.next();

    if prefix.is_empty() || parts.next().is_some() {
        return None;
    }

    let (mediator_pid, tab_id) = match third {
        Some(tab_id_str) => (Some(second.parse().ok()?), tab_id_str.parse().ok()?),
        None => (None, second.parse().ok()?),
    };

    Some(TabRef {
        prefix,
        mediator_pid,
        tab_id,
    })
}

/// Parse a window ID.
///
/// Accepts either the current `prefix.pid.w.window_id` format or the legacy
/// `prefix.w.window_id` shorthand for single-mediator setups.
pub fn parse_window_id(s: &str) -> Option<WindowRef<'_>> {
    let parts = s.split('.').collect::<Vec<_>>();

    match parts.as_slice() {
        [prefix, pid_str, "w", window_id_str] if !prefix.is_empty() => Some(WindowRef {
            prefix,
            mediator_pid: Some(pid_str.parse().ok()?),
            window_id: window_id_str.parse().ok()?,
        }),
        [prefix, "w", window_id_str] if !prefix.is_empty() => Some(WindowRef {
            prefix,
            mediator_pid: None,
            window_id: window_id_str.parse().ok()?,
        }),
        _ => None,
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
    fn kill(pid: i32, sig: i32) -> i32;
}

/// Check if a PID is alive.
/// `kill(pid, 0)` works on Unix even when `/proc` is absent (for example macOS).
pub fn is_pid_alive(pid: u32) -> bool {
    if pid > i32::MAX as u32 {
        return false;
    }

    match unsafe { kill(pid as i32, 0) } {
        0 => true,
        _ => matches!(std::io::Error::last_os_error().raw_os_error(), Some(1)),
    }
}

/// Browser info for native messaging manifest installation.
pub struct BrowserManifestInfo {
    pub name: &'static str,
    pub config_dir: &'static str,
    pub manifest_subdir: &'static str,
    pub is_firefox: bool,
}

/// Chromium extension ID derived from the extension manifest `key`.
pub const CHROME_EXTENSION_ID: &str = "nddbmnpippfilnjoebpcnfbpebnllbgo";
pub const BROWSERS: &[BrowserManifestInfo] = &[
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "chrome",
        config_dir: ".config/google-chrome",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "chrome",
        config_dir: "Library/Application Support/Google/Chrome",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "brave",
        config_dir: ".config/BraveSoftware/Brave-Browser",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "brave",
        config_dir: "Library/Application Support/BraveSoftware/Brave-Browser",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "chromium",
        config_dir: ".config/chromium",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "chromium",
        config_dir: "Library/Application Support/Chromium",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "orion",
        config_dir: "Library/Application Support/Orion",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "edge",
        config_dir: ".config/microsoft-edge",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "edge",
        config_dir: "Library/Application Support/Microsoft Edge",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "vivaldi",
        config_dir: ".config/vivaldi",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "vivaldi",
        config_dir: "Library/Application Support/Vivaldi",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: false,
    },
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "firefox",
        config_dir: ".mozilla",
        manifest_subdir: "native-messaging-hosts",
        is_firefox: true,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "firefox",
        config_dir: "Library/Application Support/Mozilla",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: true,
    },
    #[cfg(target_os = "linux")]
    BrowserManifestInfo {
        name: "zen",
        config_dir: ".zen",
        manifest_subdir: "native-messaging-hosts",
        is_firefox: true,
    },
    #[cfg(target_os = "macos")]
    BrowserManifestInfo {
        name: "zen",
        config_dir: "Library/Application Support/zen",
        manifest_subdir: "NativeMessagingHosts",
        is_firefox: true,
    },
];

pub const NATIVE_HOST_NAME: &str = "rustab_mediator";
pub const FIREFOX_EXTENSION_ID: &str = "rustab@rustab.dev";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn socket_dir_uses_effective_uid() {
        let uid = unsafe { geteuid() };
        assert_eq!(socket_dir(), PathBuf::from(format!("/tmp/rustab-{uid}")));
    }

    #[test]
    fn impossible_pid_is_not_alive() {
        assert!(!is_pid_alive(u32::MAX));
    }

    #[cfg(unix)]
    #[test]
    fn validates_private_socket_dirs() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_test_dir("private-socket-dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(validate_socket_dir(&dir).is_ok());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_shared_socket_dirs() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_test_dir("shared-socket-dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = validate_socket_dir(&dir).expect_err("shared dir should be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_socket_names() {
        assert_eq!(
            parse_socket_name("brave-123.sock"),
            Some(("brave".to_string(), 123))
        );
    }

    #[test]
    fn parses_legacy_tab_ids() {
        assert_eq!(
            parse_tab_id("b.42"),
            Some(TabRef {
                prefix: "b",
                mediator_pid: None,
                tab_id: 42,
            })
        );
    }

    #[test]
    fn parses_full_tab_ids() {
        assert_eq!(
            parse_tab_id("b.12345.42"),
            Some(TabRef {
                prefix: "b",
                mediator_pid: Some(12345),
                tab_id: 42,
            })
        );
    }

    #[test]
    fn formats_full_tab_ids() {
        assert_eq!(format_tab_id("b", 12345, 42), "b.12345.42");
    }

    #[test]
    fn parses_full_window_ids() {
        assert_eq!(
            parse_window_id("b.12345.w.42"),
            Some(WindowRef {
                prefix: "b",
                mediator_pid: Some(12345),
                window_id: 42,
            })
        );
    }

    #[test]
    fn parses_legacy_window_ids() {
        assert_eq!(
            parse_window_id("b.w.42"),
            Some(WindowRef {
                prefix: "b",
                mediator_pid: None,
                window_id: 42,
            })
        );
    }

    #[test]
    fn rejects_malformed_window_ids() {
        assert_eq!(parse_window_id("b.12345.42"), None);
        assert_eq!(parse_window_id("b.12345.window.42"), None);
        assert_eq!(parse_window_id(".12345.w.42"), None);
    }

    #[test]
    fn formats_full_window_ids() {
        assert_eq!(format_window_id("b", 12345, 42), "b.12345.w.42");
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustab-protocol-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir(&dir).expect("create temp dir");
        dir
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_browser_paths_match_expected_locations() {
        let firefox = BROWSERS
            .iter()
            .find(|browser| browser.name == "firefox")
            .unwrap();
        let brave = BROWSERS
            .iter()
            .find(|browser| browser.name == "brave")
            .unwrap();
        let zen = BROWSERS
            .iter()
            .find(|browser| browser.name == "zen")
            .unwrap();

        assert_eq!(brave.config_dir, ".config/BraveSoftware/Brave-Browser");
        assert_eq!(firefox.config_dir, ".mozilla");
        assert_eq!(zen.manifest_subdir, "native-messaging-hosts");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_browser_paths_match_expected_locations() {
        let firefox = BROWSERS
            .iter()
            .find(|browser| browser.name == "firefox")
            .unwrap();
        let brave = BROWSERS
            .iter()
            .find(|browser| browser.name == "brave")
            .unwrap();
        let orion = BROWSERS
            .iter()
            .find(|browser| browser.name == "orion")
            .unwrap();
        let zen = BROWSERS
            .iter()
            .find(|browser| browser.name == "zen")
            .unwrap();

        assert_eq!(
            brave.config_dir,
            "Library/Application Support/BraveSoftware/Brave-Browser"
        );
        assert_eq!(orion.config_dir, "Library/Application Support/Orion");
        assert_eq!(firefox.config_dir, "Library/Application Support/Mozilla");
        assert_eq!(zen.manifest_subdir, "NativeMessagingHosts");
    }
}
