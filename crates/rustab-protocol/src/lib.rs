use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const REQUEST_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_BROWSER_REQUEST_TIMEOUT_SECS: u64 = REQUEST_TIMEOUT_SECS;
pub const DEFAULT_CLIENT_REQUEST_TIMEOUT_HEADROOM_SECS: u64 = 5;
pub const ORION_BROWSER_REQUEST_TIMEOUT_SECS: u64 = 60;
pub const ORION_CLIENT_REQUEST_TIMEOUT_HEADROOM_SECS: u64 = 10;
pub const BROWSER_REQUEST_TIMEOUT_ENV: &str = "RUSTAB_BROWSER_REQUEST_TIMEOUT_SECS";
pub const CLIENT_REQUEST_TIMEOUT_ENV: &str = "RUSTAB_CLIENT_REQUEST_TIMEOUT_SECS";
pub const LIST_TABS_METHOD: &str = "list_tabs";
pub const LIST_WINDOWS_METHOD: &str = "list_windows";
pub const CLOSE_TABS_METHOD: &str = "close_tabs";
pub const ACTIVATE_TAB_METHOD: &str = "activate_tab";
pub const OPEN_TAB_METHOD: &str = "open_tab";
pub const MOVE_TABS_METHOD: &str = "move_tabs";

const DEFAULT_REQUEST_ID: u64 = 1;
const MAX_INBOUND_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_OUTBOUND_MESSAGE_BYTES: usize = 1024 * 1024;

pub fn browser_request_timeout_for(browser: &str) -> Duration {
    Duration::from_secs(browser_request_timeout_secs_from_value(
        browser,
        std::env::var(BROWSER_REQUEST_TIMEOUT_ENV).ok().as_deref(),
    ))
}

pub fn client_request_timeout_for(browser: &str) -> Duration {
    let browser_override = std::env::var(BROWSER_REQUEST_TIMEOUT_ENV).ok();
    let client_override = std::env::var(CLIENT_REQUEST_TIMEOUT_ENV).ok();
    Duration::from_secs(client_request_timeout_secs_from_values(
        browser,
        client_override.as_deref(),
        browser_override.as_deref(),
    ))
}

fn browser_request_timeout_secs_from_value(browser: &str, value: Option<&str>) -> u64 {
    timeout_secs_from_value(value, default_browser_request_timeout_secs(browser))
}

fn client_request_timeout_secs_from_values(
    browser: &str,
    client_override: Option<&str>,
    browser_override: Option<&str>,
) -> u64 {
    if let Some(client_timeout) = parse_positive_timeout_secs(client_override) {
        return client_timeout;
    }

    browser_request_timeout_secs_from_value(browser, browser_override)
        .saturating_add(client_timeout_headroom_secs(browser))
}

fn client_timeout_headroom_secs(browser: &str) -> u64 {
    if browser.eq_ignore_ascii_case("orion") {
        ORION_CLIENT_REQUEST_TIMEOUT_HEADROOM_SECS
    } else {
        DEFAULT_CLIENT_REQUEST_TIMEOUT_HEADROOM_SECS
    }
}

fn default_browser_request_timeout_secs(browser: &str) -> u64 {
    if browser.eq_ignore_ascii_case("orion") {
        ORION_BROWSER_REQUEST_TIMEOUT_SECS
    } else {
        DEFAULT_BROWSER_REQUEST_TIMEOUT_SECS
    }
}

fn timeout_secs_from_value(value: Option<&str>, default_secs: u64) -> u64 {
    parse_positive_timeout_secs(value).unwrap_or(default_secs)
}

fn parse_positive_timeout_secs(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
}

/// Read a native-messaging-framed JSON message.
///
/// Wire format: 4-byte little-endian u32 length prefix, then UTF-8 JSON payload.
/// This is the same framing used by Chrome/Firefox native messaging over stdio
/// and by our Unix socket protocol.
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<serde_json::Value> {
    read_message_with_recovery(reader, false).await
}

/// Read a native-messaging-framed JSON message, tolerating short length
/// prefixes from buggy native-messaging hosts.
///
/// Orion 1.0.x appears to frame extension-to-native messages using the JS
/// string length rather than the UTF-8 byte length. When tab titles or URLs
/// contain non-ASCII characters, the declared length ends in the middle of a
/// JSON string. In that one narrow case, keep reading bytes until the JSON
/// payload becomes complete.
pub async fn read_message_lenient<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<serde_json::Value> {
    read_message_with_recovery(reader, true).await
}

/// Read a browser-to-native native-messaging frame.
///
/// Orion 1.0.x is the only browser with a known short UTF-8 length-prefix bug;
/// keep all other browser traffic on the strict reader.
pub async fn read_browser_message<R: AsyncRead + Unpin>(
    browser: &str,
    reader: &mut R,
) -> io::Result<serde_json::Value> {
    if browser.eq_ignore_ascii_case("orion") {
        read_message_lenient(reader).await
    } else {
        read_message(reader).await
    }
}

async fn read_message_with_recovery<R: AsyncRead + Unpin>(
    reader: &mut R,
    recover_short_length_prefix: bool,
) -> io::Result<serde_json::Value> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty message"));
    }
    if len > MAX_INBOUND_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds 64 MiB",
        ));
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;

    loop {
        match serde_json::from_slice(&buf) {
            Ok(value) => return Ok(value),
            Err(error)
                if recover_short_length_prefix
                    && error.is_eof()
                    && buf.len() < MAX_INBOUND_MESSAGE_BYTES =>
            {
                let mut extra = [0u8; 1];
                reader.read_exact(&mut extra).await?;
                buf.push(extra[0]);
            }
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
}

/// Write a native-messaging-framed JSON message.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &serde_json::Value,
) -> io::Result<()> {
    let payload = serde_json::to_vec(msg)?;

    if payload.len() > MAX_OUTBOUND_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds 1 MiB outbound limit",
        ));
    }

    let len = u32::try_from(payload.len())
        .expect("payload length is capped below u32::MAX")
        .to_le_bytes();
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
    /// Construct a request with the default CLI request ID.
    ///
    /// The CLI sends one request per socket connection, and the mediator rewrites
    /// this ID to a process-wide unique value before forwarding it to the browser.
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self::with_id(DEFAULT_REQUEST_ID, method, params)
    }

    pub fn with_id(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            id,
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
    pub fn into_result_for_request(self, request_id: u64) -> Result<T, String> {
        if self.id != request_id {
            return Err(format!(
                "response id {} did not match request id {}",
                self.id, request_id
            ));
        }

        self.into_result()
    }

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
        let uid = effective_uid();
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

    let uid = effective_uid();
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
        other => {
            debug_assert!(
                false,
                "unknown browser {other:?} — add a prefix to browser_prefix"
            );
            "u"
        }
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
    let mut parts = s.split('.');
    let prefix = parts.next()?;
    if prefix.is_empty() {
        return None;
    }

    match (parts.next()?, parts.next()?, parts.next(), parts.next()) {
        (pid_str, "w", Some(window_id_str), None) => Some(WindowRef {
            prefix,
            mediator_pid: Some(pid_str.parse().ok()?),
            window_id: window_id_str.parse().ok()?,
        }),
        ("w", window_id_str, None, None) => Some(WindowRef {
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

#[cfg(unix)]
const EPERM: i32 = 1;

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe { geteuid() }
}

#[cfg(unix)]
fn send_signal(pid: i32, signal: i32) -> i32 {
    unsafe { kill(pid, signal) }
}

/// Check if a PID is alive.
///
/// `kill(pid, 0)` returns 0 when a concrete process exists and is signallable,
/// -1/ESRCH when it does not exist, and -1/EPERM when it exists but is owned by
/// another user. Both 0 and EPERM mean "alive." PID 0 is excluded because it
/// targets the caller's process group rather than a socket owner's process.
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };

    match send_signal(pid, 0) {
        0 => true,
        // EPERM: process exists but we lack permission to signal it.
        _ => matches!(std::io::Error::last_os_error().raw_os_error(), Some(EPERM)),
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

    fn short_utf8_frame(message: &serde_json::Value) -> Vec<u8> {
        let payload = serde_json::to_vec(message).expect("json payload");
        let short_len = String::from_utf8(payload.clone())
            .expect("utf8 json")
            .chars()
            .count();
        assert!(short_len < payload.len());

        let mut framed = Vec::new();
        framed.extend_from_slice(&(short_len as u32).to_le_bytes());
        framed.extend_from_slice(&payload);
        framed
    }

    #[tokio::test]
    async fn lenient_read_recovers_short_utf8_length_prefix() {
        let message = serde_json::json!({
            "id": 7,
            "result": {"title": "café tab", "url": "https://example.com/ação"}
        });
        let framed = short_utf8_frame(&message);
        let mut reader = std::io::Cursor::new(framed);
        let parsed = read_message_lenient(&mut reader).await.expect("parsed");
        assert_eq!(parsed, message);
    }

    #[tokio::test]
    async fn strict_read_rejects_short_utf8_length_prefix() {
        let message = serde_json::json!({"id": 7, "result": {"title": "café tab"}});
        let framed = short_utf8_frame(&message);

        let mut reader = std::io::Cursor::new(framed);
        let error = read_message(&mut reader)
            .await
            .expect_err("strict parser rejects frame");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn browser_read_uses_lenient_reader_for_orion() {
        let message = serde_json::json!({"id": 7, "result": {"title": "café tab"}});
        let framed = short_utf8_frame(&message);

        let mut reader = std::io::Cursor::new(framed);
        let parsed = read_browser_message("Orion", &mut reader)
            .await
            .expect("orion reader recovers frame");
        assert_eq!(parsed, message);
    }

    #[tokio::test]
    async fn browser_read_uses_strict_reader_for_non_orion() {
        let message = serde_json::json!({"id": 7, "result": {"title": "café tab"}});
        let framed = short_utf8_frame(&message);

        let mut reader = std::io::Cursor::new(framed);
        let error = read_browser_message("chrome", &mut reader)
            .await
            .expect_err("non-orion reader rejects frame");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn current_process_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn socket_dir_uses_effective_uid() {
        let uid = effective_uid();
        assert_eq!(socket_dir(), PathBuf::from(format!("/tmp/rustab-{uid}")));
    }

    #[test]
    fn impossible_pid_is_not_alive() {
        assert!(!is_pid_alive(u32::MAX));
    }

    #[test]
    fn pid_zero_is_not_alive() {
        assert!(!is_pid_alive(0));
    }

    #[test]
    fn request_with_id_preserves_caller_id() {
        let request = RpcRequest::with_id(42, LIST_TABS_METHOD, serde_json::json!({}));

        assert_eq!(request.id, 42);
        assert_eq!(request.method, LIST_TABS_METHOD);
    }

    #[test]
    fn orion_uses_longer_live_bridge_timeouts() {
        assert!(
            default_browser_request_timeout_secs("orion")
                > default_browser_request_timeout_secs("brave")
        );
        assert!(
            client_request_timeout_secs_from_values("orion", None, None)
                > client_request_timeout_secs_from_values("brave", None, None)
        );
    }

    #[test]
    fn client_timeout_exceeds_browser_timeout() {
        for browser in ["brave", "firefox", "orion"] {
            assert!(
                client_request_timeout_secs_from_values(browser, None, None)
                    > default_browser_request_timeout_secs(browser),
                "client timeout should outlast mediator/browser timeout for {browser}"
            );
        }
    }

    #[test]
    fn client_timeout_tracks_browser_timeout_override_when_client_override_is_unset() {
        assert_eq!(
            client_request_timeout_secs_from_values("orion", None, Some("120")),
            130
        );
        assert_eq!(
            client_request_timeout_secs_from_values("brave", None, Some("120")),
            125
        );
    }

    #[test]
    fn explicit_client_timeout_override_is_respected() {
        assert_eq!(
            client_request_timeout_secs_from_values("orion", Some("65"), Some("120")),
            65
        );
    }

    #[test]
    fn timeout_override_parser_rejects_zero_and_invalid_values() {
        assert_eq!(timeout_secs_from_value(Some("25"), 10), 25);
        assert_eq!(timeout_secs_from_value(Some("0"), 10), 10);
        assert_eq!(timeout_secs_from_value(Some("nope"), 10), 10);
        assert_eq!(timeout_secs_from_value(None, 10), 10);
    }

    #[test]
    fn response_id_mismatch_is_rejected() {
        let response = RpcResponse {
            id: 2,
            result: Some("ok".to_string()),
            error: None,
        };

        let error = response
            .into_result_for_request(1)
            .expect_err("mismatched response id should fail");

        assert_eq!(error, "response id 2 did not match request id 1");
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
