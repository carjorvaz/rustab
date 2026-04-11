use clap::{Parser, Subcommand, ValueEnum};
use rustab_protocol::{
    browser_prefix, is_pid_alive, parse_socket_name, parse_tab_id, read_message, socket_dir,
    write_message, BROWSERS, CHROME_EXTENSION_ID, FIREFOX_EXTENSION_ID, NATIVE_HOST_NAME,
};
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "rustab", about = "Browser tab management from the terminal")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Tsv,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// List open tabs across all browsers
    List {
        /// Output format
        #[arg(short, long, default_value = "tsv")]
        format: OutputFormat,
        /// Filter by browser (e.g. chrome, firefox, brave)
        #[arg(short, long)]
        browser: Option<String>,
    },
    /// Close tabs by ID (prefix.id format, from args or stdin)
    Close {
        /// Tab IDs to close; reads from stdin if none given
        tab_ids: Vec<String>,
    },
    /// Activate (focus) a tab by ID
    Activate {
        /// Tab ID (prefix.id format)
        tab_id: String,
    },
    /// Open a URL in a new tab
    Open {
        /// URL to open
        url: String,
        /// Target browser (uses first available if not specified)
        #[arg(short, long)]
        browser: Option<String>,
    },
    /// Show connected browsers
    Clients,
    /// Install native messaging manifests for detected browsers
    Install {
        /// Path to rustab-mediator binary (auto-detected if not specified)
        #[arg(long)]
        mediator_path: Option<PathBuf>,
        /// Chrome extension ID (for development/sideloaded extensions)
        #[arg(long)]
        chrome_extension_id: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    // Reset SIGPIPE so piping to `head` etc. exits cleanly
    // instead of panicking (Rust sets SIG_IGN by default).
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    let code = match cli.command {
        Command::List { format, browser } => cmd_list(&format, browser.as_deref()).await,
        Command::Close { tab_ids } => cmd_close(tab_ids).await,
        Command::Activate { tab_id } => cmd_activate(&tab_id).await,
        Command::Open { url, browser } => cmd_open(&url, browser.as_deref()).await,
        Command::Clients => cmd_clients(),
        Command::Install {
            mediator_path,
            chrome_extension_id,
        } => cmd_install(mediator_path, chrome_extension_id),
    };

    std::process::exit(code);
}

// --- Socket discovery ---

struct BrowserSocket {
    browser: String,
    path: PathBuf,
}

fn discover_sockets(browser_filter: Option<&str>) -> Vec<BrowserSocket> {
    let dir = socket_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let (browser, pid) = parse_socket_name(&name)?;

            if !is_pid_alive(pid) {
                return None;
            }

            if let Some(filter) = browser_filter {
                if browser != filter {
                    return None;
                }
            }

            Some(BrowserSocket {
                browser,
                path: entry.path(),
            })
        })
        .collect()
}

async fn send_request(socket_path: &std::path::Path, request: &Value) -> Result<Value, String> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let (mut reader, mut writer) = stream.into_split();

    write_message(&mut writer, request)
        .await
        .map_err(|e| format!("write: {e}"))?;

    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_message(&mut reader),
    )
    .await
    .map_err(|_| "timeout".to_string())?
    .map_err(|e| format!("read: {e}"))
}

fn find_socket_for_prefix<'a>(
    sockets: &'a [BrowserSocket],
    prefix: &str,
) -> Option<&'a BrowserSocket> {
    sockets
        .iter()
        .find(|s| browser_prefix(&s.browser) == prefix)
}

/// Collect tab IDs from args, or read from stdin (one per line, first
/// whitespace-delimited field — so `rustab list | rustab close` works).
fn collect_tab_ids(mut args: Vec<String>) -> Vec<String> {
    if !args.is_empty() {
        return args;
    }

    // Only read stdin if it's not a terminal (i.e. piped)
    if atty_stdin() {
        return args;
    }

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Take the first field (tab ID from TSV output)
        if let Some(id) = line.split('\t').next() {
            let id = id.trim();
            if !id.is_empty() {
                args.push(id.to_string());
            }
        }
    }

    args
}

fn atty_stdin() -> bool {
    std::io::stdin().is_terminal()
}

// --- Commands ---

async fn cmd_list(format: &OutputFormat, browser_filter: Option<&str>) -> i32 {
    let sockets = discover_sockets(browser_filter);

    if sockets.is_empty() {
        eprintln!("No browsers connected. Is the extension installed and rustab-mediator running?");
        return 1;
    }

    let mut all_tabs: Vec<(String, Value)> = Vec::new();

    for sock in &sockets {
        let request = json!({"id": 1, "method": "list_tabs"});
        match send_request(&sock.path, &request).await {
            Ok(response) => {
                if let Some(tabs) = response.get("result").and_then(|r| r.as_array()) {
                    for tab in tabs {
                        all_tabs.push((sock.browser.clone(), tab.clone()));
                    }
                } else if let Some(err) = response.get("error") {
                    eprintln!("{}: {}", sock.browser, err);
                }
            }
            Err(e) => eprintln!("{}: {e}", sock.browser),
        }
    }

    // Sort by window so tabs from the same window are grouped together
    all_tabs.sort_by_key(|(_, tab)| tab.get("window_id").and_then(|v| v.as_u64()).unwrap_or(0));

    match format {
        OutputFormat::Json => {
            let out: Vec<Value> = all_tabs
                .iter()
                .map(|(browser, tab)| {
                    let prefix = browser_prefix(browser);
                    let id = tab.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let window_id = tab.get("window_id").and_then(|v| v.as_u64()).unwrap_or(0);
                    json!({
                        "id": format!("{prefix}.{id}"),
                        "browser": browser,
                        "window_id": window_id,
                        "title": tab.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "url": tab.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                        "active": tab.get("active").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        OutputFormat::Tsv => {
            for (browser, tab) in &all_tabs {
                let prefix = browser_prefix(browser);
                let id = tab.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = tab.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let url = tab.get("url").and_then(|v| v.as_str()).unwrap_or("");
                println!("{prefix}.{id}\t{title}\t{url}");
            }
        }
    }

    0
}

async fn cmd_close(tab_ids: Vec<String>) -> i32 {
    let tab_ids = collect_tab_ids(tab_ids);

    if tab_ids.is_empty() {
        eprintln!("No tab IDs provided. Pass as arguments or pipe from `rustab list`.");
        return 1;
    }

    // Group tab IDs by browser prefix
    let mut by_browser: std::collections::HashMap<String, Vec<u64>> =
        std::collections::HashMap::new();

    for id_str in &tab_ids {
        match parse_tab_id(id_str) {
            Some((prefix, id)) => {
                by_browser.entry(prefix.to_string()).or_default().push(id);
            }
            None => {
                eprintln!("Invalid tab ID format: {id_str} (expected prefix.number, e.g. c.123)");
                return 1;
            }
        }
    }

    let sockets = discover_sockets(None);
    let mut failed = false;

    for (prefix, ids) in &by_browser {
        let Some(sock) = find_socket_for_prefix(&sockets, prefix) else {
            eprintln!("No browser connected for prefix '{prefix}'");
            failed = true;
            continue;
        };

        let request = json!({"id": 1, "method": "close_tabs", "params": {"tab_ids": ids}});
        match send_request(&sock.path, &request).await {
            Ok(response) => {
                if let Some(err) = response.get("error") {
                    eprintln!("{}: {}", sock.browser, err);
                    failed = true;
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", sock.browser);
                failed = true;
            }
        }
    }

    i32::from(failed)
}

async fn cmd_activate(tab_id: &str) -> i32 {
    let Some((prefix, id)) = parse_tab_id(tab_id) else {
        eprintln!("Invalid tab ID format: {tab_id} (expected prefix.number, e.g. c.123)");
        return 1;
    };

    let sockets = discover_sockets(None);
    let Some(sock) = find_socket_for_prefix(&sockets, prefix) else {
        eprintln!("No browser connected for prefix '{prefix}'");
        return 1;
    };

    let request = json!({"id": 1, "method": "activate_tab", "params": {"tab_id": id}});
    match send_request(&sock.path, &request).await {
        Ok(response) => {
            if let Some(err) = response.get("error") {
                eprintln!("{}: {}", sock.browser, err);
                return 1;
            }
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", sock.browser);
            1
        }
    }
}

async fn cmd_open(url: &str, browser_filter: Option<&str>) -> i32 {
    let sockets = discover_sockets(browser_filter);

    let Some(sock) = sockets.first() else {
        eprintln!("No browsers connected");
        return 1;
    };

    let request = json!({"id": 1, "method": "open_tab", "params": {"url": url}});
    match send_request(&sock.path, &request).await {
        Ok(response) => {
            if let Some(err) = response.get("error") {
                eprintln!("{}: {}", sock.browser, err);
                return 1;
            }
            if let Some(result) = response.get("result") {
                let id = result.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let prefix = browser_prefix(&sock.browser);
                println!("{prefix}.{id}");
            }
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", sock.browser);
            1
        }
    }
}

fn cmd_clients() -> i32 {
    let sockets = discover_sockets(None);

    if sockets.is_empty() {
        eprintln!("No browsers connected.");
        return 1;
    }

    for sock in &sockets {
        let prefix = browser_prefix(&sock.browser);
        println!("{prefix}\t{}\t{}", sock.browser, sock.path.display());
    }

    0
}

fn cmd_install(mediator_path: Option<PathBuf>, chrome_extension_id: Option<String>) -> i32 {
    let mediator = mediator_path.unwrap_or_else(|| {
        // Try to find rustab-mediator next to this binary
        if let Ok(exe) = std::env::current_exe() {
            let sibling = exe.with_file_name("rustab-mediator");
            if sibling.exists() {
                return sibling;
            }
        }
        // Search PATH
        if let Some(path) = find_in_path("rustab-mediator") {
            return path;
        }
        eprintln!("Could not find rustab-mediator. Use --mediator-path to specify.");
        std::process::exit(1);
    });

    let mediator_abs = match std::fs::canonicalize(&mediator) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Cannot resolve mediator path {}: {e}", mediator.display());
            return 1;
        }
    };

    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => {
            eprintln!("$HOME not set");
            return 1;
        }
    };

    let using_default_chrome_extension_id = chrome_extension_id.is_none();
    let chrome_ext_id = chrome_extension_id.unwrap_or_else(|| CHROME_EXTENSION_ID.to_string());

    let mut installed = 0;

    for browser in BROWSERS {
        let config_path = PathBuf::from(&home).join(browser.config_dir);
        if !config_path.exists() {
            continue;
        }

        let manifest_dir = config_path.join(browser.manifest_subdir);
        if let Err(e) = std::fs::create_dir_all(&manifest_dir) {
            eprintln!("{}: failed to create manifest dir: {e}", browser.name);
            continue;
        }

        let manifest = if browser.is_firefox {
            build_firefox_manifest(&mediator_abs)
        } else {
            build_chrome_manifest(&mediator_abs, &chrome_ext_id)
        };

        let manifest_path = manifest_dir.join(format!("{NATIVE_HOST_NAME}.json"));
        match std::fs::write(&manifest_path, manifest) {
            Ok(()) => {
                println!("{}: installed {}", browser.name, manifest_path.display());
                installed += 1;
            }
            Err(e) => eprintln!("{}: failed to write manifest: {e}", browser.name),
        }
    }

    if installed == 0 {
        eprintln!("No browsers detected. Check that browser config directories exist.");
        return 1;
    }

    println!("\nInstalled manifests for {installed} browser(s).");
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

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

fn build_chrome_manifest(mediator_path: &std::path::Path, extension_id: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "name": NATIVE_HOST_NAME,
        "description": "rustab native messaging host",
        "path": mediator_path.to_str().unwrap(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")]
    }))
    .unwrap()
}

fn build_firefox_manifest(mediator_path: &std::path::Path) -> String {
    serde_json::to_string_pretty(&json!({
        "name": NATIVE_HOST_NAME,
        "description": "rustab native messaging host",
        "path": mediator_path.to_str().unwrap(),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID]
    }))
    .unwrap()
}
