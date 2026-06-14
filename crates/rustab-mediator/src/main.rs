use rustab_protocol::{
    browser_request_timeout_for, is_pid_alive, prepare_socket_dir, read_message,
    read_message_lenient, socket_path, write_message,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Global monotonic counter for request IDs.
/// Prevents collisions when multiple CLI clients send concurrent requests.
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

type PendingResponses = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

// Log to stderr (stdout is reserved for native messaging).
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("[rustab-mediator] {}", format_args!($($arg)*))
    };
}

#[tokio::main]
async fn main() {
    let browser = detect_browser();
    let request_timeout = browser_request_timeout_for(&browser);
    let pid = std::process::id();
    log!(
        "starting (browser={browser}, pid={pid}, request_timeout={}s)",
        request_timeout.as_secs()
    );

    let sock_dir = match prepare_socket_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log!("failed to prepare socket dir: {e}");
            std::process::exit(1);
        }
    };

    cleanup_stale_sockets(&sock_dir);

    let sock_path = socket_path(&browser, pid);
    let _ = std::fs::remove_file(&sock_path);

    let listener = match UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            log!("failed to bind socket: {e}");
            std::process::exit(1);
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));
    }

    log!("listening on {}", sock_path.display());

    // Channel: messages to send to browser extension via stdout
    let (browser_tx, mut browser_rx) = mpsc::channel::<Value>(64);

    // Pending responses: request_id -> oneshot sender
    let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));

    // Task: write to stdout (native messaging to browser extension)
    let stdout_handle = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(msg) = browser_rx.recv().await {
            if let Err(e) = write_message(&mut stdout, &msg).await {
                log!("stdout write error: {e}");
                break;
            }
        }
    });

    // Task: read from stdin (native messaging from browser extension)
    let pending_for_stdin = pending.clone();
    let stdin_browser = browser.clone();
    let stdin_handle = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        loop {
            match read_browser_message(&stdin_browser, &mut stdin).await {
                Ok(msg) => {
                    if let Some(id) = request_id(&msg) {
                        let mut map = pending_for_stdin.lock().await;
                        if let Some(sender) = map.remove(&id) {
                            let _ = sender.send(msg);
                        }
                    }
                }
                Err(e) => {
                    log!("stdin closed: {e}");
                    break;
                }
            }
        }
    });

    // Task: accept CLI clients on Unix socket
    let accept_handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tx = browser_tx.clone();
                    let pend = pending.clone();
                    tokio::spawn(handle_client(stream, tx, pend, request_timeout));
                }
                Err(e) => {
                    log!("accept error: {e}");
                    break;
                }
            }
        }
    });

    // Shutdown when stdin closes (browser exited) or socket accept fails
    tokio::select! {
        _ = stdin_handle => log!("browser disconnected, shutting down"),
        _ = accept_handle => log!("socket accept failed, shutting down"),
        _ = stdout_handle => log!("stdout closed, shutting down"),
        _ = tokio::signal::ctrl_c() => log!("interrupted, shutting down"),
    }

    let _ = std::fs::remove_file(&sock_path);
}

fn request_id(message: &Value) -> Option<u64> {
    message.get("id").and_then(Value::as_u64)
}

async fn read_browser_message<R>(browser: &str, reader: &mut R) -> std::io::Result<Value>
where
    R: AsyncRead + Unpin,
{
    if browser.eq_ignore_ascii_case("orion") {
        read_message_lenient(reader).await
    } else {
        read_message(reader).await
    }
}

fn set_request_id(message: &mut Value, id: u64) {
    message["id"] = json!(id);
}

async fn write_client_error<W>(writer: &mut W, client_id: Option<u64>, error: &str)
where
    W: AsyncWrite + Unpin,
{
    let response = match client_id {
        Some(id) => json!({"id": id, "error": error}),
        None => json!({"error": error}),
    };
    let _ = write_message(writer, &response).await;
}

async fn handle_client(
    stream: tokio::net::UnixStream,
    browser_tx: mpsc::Sender<Value>,
    pending: PendingResponses,
    request_timeout: std::time::Duration,
) {
    let (mut reader, mut writer) = stream.into_split();

    loop {
        let mut msg = match read_message(&mut reader).await {
            Ok(msg) => msg,
            Err(_) => break, // client disconnected
        };

        let Some(client_id) = request_id(&msg) else {
            write_client_error(&mut writer, None, "missing request id").await;
            continue;
        };

        // Assign a unique internal ID to prevent collisions across clients
        let internal_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        set_request_id(&mut msg, internal_id);

        // Register a oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        {
            pending.lock().await.insert(internal_id, tx);
        }

        // Forward request to browser extension
        if browser_tx.send(msg).await.is_err() {
            write_client_error(&mut writer, Some(client_id), "browser disconnected").await;
            pending.lock().await.remove(&internal_id);
            break;
        }

        // Wait for response with timeout
        let response = tokio::time::timeout(request_timeout, rx).await;
        match response {
            Ok(Ok(mut val)) => {
                // Restore the client's original ID
                set_request_id(&mut val, client_id);
                if write_message(&mut writer, &val).await.is_err() {
                    break;
                }
            }
            Ok(Err(_)) => {
                write_client_error(&mut writer, Some(client_id), "response channel dropped").await;
            }
            Err(_) => {
                pending.lock().await.remove(&internal_id);
                write_client_error(&mut writer, Some(client_id), "request timed out").await;
            }
        }
    }
}

/// Detect which browser launched us based on CLI args and parent process.
fn detect_browser() -> String {
    let args: Vec<String> = std::env::args().collect();
    let parent_process_name = parent_process_name();

    detect_browser_from_launch_context(&args, parent_process_name.as_deref()).into()
}

struct FirefoxLaunchHint {
    browser: &'static str,
    arg_substring: &'static str,
    parent_substring: &'static str,
}

const FIREFOX_LAUNCH_HINTS: &[FirefoxLaunchHint] = &[
    FirefoxLaunchHint {
        browser: "zen",
        arg_substring: ".zen",
        parent_substring: "zen",
    },
    FirefoxLaunchHint {
        browser: "firefox",
        arg_substring: ".mozilla",
        parent_substring: "firefox",
    },
];

const CHROMIUM_PARENT_HINTS: &[(&str, &str)] = &[
    ("brave", "brave"),
    ("orion", "orion"),
    ("edge", "edge"),
    ("vivaldi", "vivaldi"),
    ("chromium", "chromium"),
];

fn detect_browser_from_launch_context(
    args: &[String],
    parent_process_name: Option<&str>,
) -> &'static str {
    let parent_process_name = parent_process_name.map(str::to_lowercase);
    let parent_contains = |needle: &str| {
        parent_process_name
            .as_deref()
            .is_some_and(|name| name.contains(needle))
    };

    // Firefox-based: macOS Firefox can omit a `.mozilla` path when spawning
    // the native host, so fall back to the parent process name there.
    for hint in FIREFOX_LAUNCH_HINTS {
        if args.iter().any(|arg| arg.contains(hint.arg_substring))
            || parent_contains(hint.parent_substring)
        {
            return hint.browser;
        }
    }

    // Chromium-based: arg contains chrome-extension://
    if args.iter().any(|arg| arg.contains("chrome-extension://")) {
        for &(parent_substring, browser) in CHROMIUM_PARENT_HINTS {
            if parent_contains(parent_substring) {
                return browser;
            }
        }
        return "chrome";
    }

    "unknown"
}

/// Read the parent process name.
#[cfg(target_os = "linux")]
fn parent_process_name() -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let ppid: u32 = status
        .lines()
        .find(|l| l.starts_with("PPid:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())?;
    let comm = std::fs::read_to_string(format!("/proc/{ppid}/comm")).ok()?;
    Some(comm.trim().to_string())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn parent_process_name() -> Option<String> {
    unsafe extern "C" {
        fn getppid() -> i32;
    }

    let ppid = unsafe { getppid() };
    if ppid <= 0 {
        return None;
    }

    let ppid = ppid.to_string();
    for ps in ["/bin/ps", "/usr/bin/ps", "ps"] {
        let output = match std::process::Command::new(ps)
            .args(["-o", "comm=", "-p"])
            .arg(&ppid)
            .output()
        {
            Ok(output) if output.status.success() => output,
            _ => continue,
        };

        let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !command.is_empty() {
            return Some(command);
        }
    }

    None
}

/// Remove socket files for dead processes.
fn cleanup_stale_sockets(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some((_, pid)) = rustab_protocol::parse_socket_name(name) {
            if !is_pid_alive(pid) {
                log!("removing stale socket for pid {pid}");
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_browser_from_launch_context, read_browser_message};

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
    async fn orion_reader_recovers_short_utf8_length_prefix() {
        let message = serde_json::json!({"id": 7, "result": {"title": "café tab"}});
        let framed = short_utf8_frame(&message);

        let mut reader = std::io::Cursor::new(framed);
        let parsed = read_browser_message("Orion", &mut reader)
            .await
            .expect("orion reader recovers frame");
        assert_eq!(parsed, message);
    }

    #[tokio::test]
    async fn non_orion_reader_rejects_short_utf8_length_prefix() {
        let message = serde_json::json!({"id": 7, "result": {"title": "café tab"}});
        let framed = short_utf8_frame(&message);

        let mut reader = std::io::Cursor::new(framed);
        let error = read_browser_message("chrome", &mut reader)
            .await
            .expect_err("non-orion reader rejects frame");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn detects_firefox_from_parent_process_when_args_omit_mozilla_path() {
        let args = vec!["rustab-mediator".to_string()];

        assert_eq!(
            detect_browser_from_launch_context(
                &args,
                Some("/Applications/Firefox.app/Contents/MacOS/firefox"),
            ),
            "firefox"
        );
    }

    #[test]
    fn keeps_chromium_browser_detection_from_parent_process() {
        let args = vec![
            "rustab-mediator".to_string(),
            "chrome-extension://nddbmnpippfilnjoebpcnfbpebnllbgo/".to_string(),
        ];

        assert_eq!(
            detect_browser_from_launch_context(
                &args,
                Some("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
            ),
            "brave"
        );
    }

    #[test]
    fn detects_orion_from_parent_process_for_chromium_extension() {
        let args = vec![
            "rustab-mediator".to_string(),
            "chrome-extension://nddbmnpippfilnjoebpcnfbpebnllbgo/".to_string(),
        ];

        assert_eq!(
            detect_browser_from_launch_context(
                &args,
                Some("/Applications/Orion.app/Contents/MacOS/Orion")
            ),
            "orion"
        );
    }
}
