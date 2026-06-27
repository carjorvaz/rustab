use rustab_protocol::{
    browser_request_timeout_for, is_pid_alive, prepare_socket_dir, read_browser_message,
    read_message, write_message,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWrite;
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

    let sock_path = sock_dir.join(format!("{browser}-{pid}.sock"));
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
    mut stream: tokio::net::UnixStream,
    browser_tx: mpsc::Sender<Value>,
    pending: PendingResponses,
    request_timeout: std::time::Duration,
) {
    let mut msg = match read_message(&mut stream).await {
        Ok(msg) => msg,
        Err(_) => return, // client disconnected
    };

    let Some(client_id) = request_id(&msg) else {
        write_client_error(&mut stream, None, "missing request id").await;
        return;
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
        write_client_error(&mut stream, Some(client_id), "browser disconnected").await;
        pending.lock().await.remove(&internal_id);
        return;
    }

    // Wait for response with timeout
    let response = tokio::time::timeout(request_timeout, rx).await;
    match response {
        Ok(Ok(mut val)) => {
            // Restore the client's original ID
            set_request_id(&mut val, client_id);
            let _ = write_message(&mut stream, &val).await;
        }
        Ok(Err(_)) => {
            write_client_error(&mut stream, Some(client_id), "response channel dropped").await;
        }
        Err(_) => {
            pending.lock().await.remove(&internal_id);
            write_client_error(&mut stream, Some(client_id), "request timed out").await;
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

const CHROMIUM_PARENT_HINTS: &[&str] = &["brave", "orion", "edge", "vivaldi", "chromium"];

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
        for &browser in CHROMIUM_PARENT_HINTS {
            if parent_contains(browser) {
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
    use super::{detect_browser_from_launch_context, handle_client, PendingResponses};
    use rustab_protocol::{read_message, write_message};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::UnixStream;
    use tokio::sync::{mpsc, Mutex};

    fn pending_responses() -> PendingResponses {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[tokio::test]
    async fn client_request_is_forwarded_with_internal_id_and_response_restores_client_id() {
        let (mut client, server) = UnixStream::pair().expect("unix stream pair");
        let (browser_tx, mut browser_rx) = mpsc::channel(1);
        let pending = pending_responses();
        let task = tokio::spawn(handle_client(
            server,
            browser_tx,
            pending.clone(),
            Duration::from_secs(1),
        ));

        write_message(
            &mut client,
            &json!({"id": 42, "method": "list_tabs", "params": {"active": true}}),
        )
        .await
        .expect("client request written");

        let forwarded = browser_rx.recv().await.expect("request forwarded");
        let internal_id = forwarded["id"].as_u64().expect("internal id");
        assert_ne!(internal_id, 42);
        assert_eq!(forwarded["method"], "list_tabs");
        assert_eq!(forwarded.get("params"), Some(&json!({"active": true})));

        pending
            .lock()
            .await
            .remove(&internal_id)
            .expect("pending response registered")
            .send(json!({"id": internal_id, "result": {"ok": true}}))
            .expect("client still waiting");

        let response = read_message(&mut client)
            .await
            .expect("client response received");
        assert_eq!(response, json!({"id": 42, "result": {"ok": true}}));
        task.await.expect("client handler finished");
    }

    #[tokio::test]
    async fn request_missing_id_gets_error_without_forwarding() {
        let (mut client, server) = UnixStream::pair().expect("unix stream pair");
        let (browser_tx, mut browser_rx) = mpsc::channel(1);
        let pending = pending_responses();
        let task = tokio::spawn(handle_client(
            server,
            browser_tx,
            pending,
            Duration::from_secs(1),
        ));

        write_message(&mut client, &json!({"method": "list_tabs"}))
            .await
            .expect("client request written");

        let response = read_message(&mut client)
            .await
            .expect("client error response received");
        assert_eq!(response, json!({"error": "missing request id"}));
        assert!(matches!(
            browser_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
        task.await.expect("client handler finished");
    }

    #[tokio::test]
    async fn timed_out_request_removes_pending_response_entry() {
        let (mut client, server) = UnixStream::pair().expect("unix stream pair");
        let (browser_tx, mut browser_rx) = mpsc::channel(1);
        let pending = pending_responses();
        let task = tokio::spawn(handle_client(
            server,
            browser_tx,
            pending.clone(),
            Duration::from_millis(50),
        ));

        write_message(&mut client, &json!({"id": 7, "method": "list_tabs"}))
            .await
            .expect("client request written");

        let forwarded = browser_rx.recv().await.expect("request forwarded");
        let internal_id = forwarded["id"].as_u64().expect("internal id");
        assert!(pending.lock().await.contains_key(&internal_id));

        let response = read_message(&mut client)
            .await
            .expect("client timeout response received");
        assert_eq!(response, json!({"id": 7, "error": "request timed out"}));
        task.await.expect("client handler finished");
        assert!(!pending.lock().await.contains_key(&internal_id));
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
