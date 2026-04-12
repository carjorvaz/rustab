use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// Socket directory: `/tmp/rustab-{uid}/`
pub fn socket_dir() -> PathBuf {
    #[cfg(unix)]
    {
        let uid = unsafe { geteuid() };
        return PathBuf::from(format!("/tmp/rustab-{uid}"));
    }

    #[cfg(not(unix))]
    {
        let username = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
        PathBuf::from(format!("/tmp/rustab-{username}"))
    }
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
        "chromium" => "cr",
        "zen" => "z",
        "edge" => "e",
        "vivaldi" => "v",
        "opera" => "o",
        _ => "u",
    }
}

/// Parse a prefixed tab ID like "c.123" into (browser_prefix, numeric_id).
pub fn parse_tab_id(s: &str) -> Option<(&str, u64)> {
    let (prefix, id_str) = s.split_once('.')?;
    let id = id_str.parse().ok()?;
    Some((prefix, id))
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

    #[test]
    fn parses_socket_names() {
        assert_eq!(
            parse_socket_name("brave-123.sock"),
            Some(("brave".to_string(), 123))
        );
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
        let zen = BROWSERS
            .iter()
            .find(|browser| browser.name == "zen")
            .unwrap();

        assert_eq!(
            brave.config_dir,
            "Library/Application Support/BraveSoftware/Brave-Browser"
        );
        assert_eq!(firefox.config_dir, "Library/Application Support/Mozilla");
        assert_eq!(zen.manifest_subdir, "NativeMessagingHosts");
    }
}
