use rustab_protocol::{is_pid_alive, read_message, socket_dir, socket_path, write_message};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Global monotonic counter for request IDs.
/// Prevents collisions when multiple CLI clients send concurrent requests.
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Log to stderr (stdout is reserved for native messaging).
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("[rustab-mediator] {}", format!($($arg)*))
    };
}

#[tokio::main]
async fn main() {
    let browser = detect_browser();
    let pid = std::process::id();
    log!("starting (browser={browser}, pid={pid})");

    let sock_dir = socket_dir();

    // Create socket directory with 0700 permissions
    if let Err(e) = std::fs::create_dir_all(&sock_dir) {
        log!("failed to create socket dir: {e}");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sock_dir, std::fs::Permissions::from_mode(0o700));
    }

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
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));

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
    let stdin_handle = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        loop {
            match read_message(&mut stdin).await {
                Ok(msg) => {
                    if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
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
                    tokio::spawn(handle_client(stream, tx, pend));
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

async fn handle_client(
    stream: tokio::net::UnixStream,
    browser_tx: mpsc::Sender<Value>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
) {
    let (mut reader, mut writer) = stream.into_split();

    loop {
        let mut msg = match read_message(&mut reader).await {
            Ok(msg) => msg,
            Err(_) => break, // client disconnected
        };

        let client_id = match msg.get("id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => {
                let _ = write_message(&mut writer, &json!({"error": "missing request id"})).await;
                continue;
            }
        };

        // Assign a unique internal ID to prevent collisions across clients
        let internal_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        msg["id"] = json!(internal_id);

        // Register a oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        {
            pending.lock().await.insert(internal_id, tx);
        }

        // Forward request to browser extension
        if browser_tx.send(msg).await.is_err() {
            let _ = write_message(
                &mut writer,
                &json!({"id": client_id, "error": "browser disconnected"}),
            )
            .await;
            pending.lock().await.remove(&internal_id);
            break;
        }

        // Wait for response with timeout
        let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx).await;
        match response {
            Ok(Ok(mut val)) => {
                // Restore the client's original ID
                val["id"] = json!(client_id);
                if write_message(&mut writer, &val).await.is_err() {
                    break;
                }
            }
            Ok(Err(_)) => {
                let _ = write_message(
                    &mut writer,
                    &json!({"id": client_id, "error": "response channel dropped"}),
                )
                .await;
            }
            Err(_) => {
                pending.lock().await.remove(&internal_id);
                let _ = write_message(
                    &mut writer,
                    &json!({"id": client_id, "error": "request timed out"}),
                )
                .await;
            }
        }
    }
}

/// Detect which browser launched us based on CLI args and parent process.
fn detect_browser() -> String {
    let args: Vec<String> = std::env::args().collect();

    // Firefox-based: arg contains .mozilla or .zen path
    if args.iter().any(|a| a.contains(".zen")) {
        return "zen".into();
    }
    if args.iter().any(|a| a.contains(".mozilla")) {
        return "firefox".into();
    }

    // Chromium-based: arg contains chrome-extension://
    if args.iter().any(|a| a.contains("chrome-extension://")) {
        if let Some(name) = parent_process_name() {
            let lower = name.to_lowercase();
            if lower.contains("brave") {
                return "brave".into();
            }
            if lower.contains("edge") {
                return "edge".into();
            }
            if lower.contains("vivaldi") {
                return "vivaldi".into();
            }
            if lower.contains("opera") {
                return "opera".into();
            }
            if lower.contains("chromium") {
                return "chromium".into();
            }
        }
        return "chrome".into();
    }

    "unknown".into()
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
