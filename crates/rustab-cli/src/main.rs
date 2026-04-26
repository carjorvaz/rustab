mod synced;

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use rustab_protocol::{
    browser_prefix, format_tab_id, format_window_id, is_pid_alive, parse_socket_name, parse_tab_id,
    parse_window_id, read_message, socket_dir, write_message, TabRef, WindowRef, BROWSERS,
    CHROME_EXTENSION_ID, FIREFOX_EXTENSION_ID, NATIVE_HOST_NAME,
};
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use synced::SyncedTab;
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
    /// List browser windows
    Windows {
        /// Output format
        #[arg(short, long, default_value = "tsv")]
        format: OutputFormat,
        /// Filter by browser (e.g. chrome, firefox, brave)
        #[arg(short, long)]
        browser: Option<String>,
    },
    /// Close tabs by ID (`prefix.pid.id`, or legacy `prefix.id`)
    Close {
        /// Tab IDs to close; reads from stdin if none given
        tab_ids: Vec<String>,
    },
    /// Move tabs to another window
    #[command(group(ArgGroup::new("target").required(true).args(["to_window", "to_tab"])))]
    Move {
        /// Target window ID (`prefix.pid.w.id`, `prefix.w.id`, or raw browser window ID)
        #[arg(long, value_name = "WINDOW_ID", conflicts_with = "to_tab")]
        to_window: Option<String>,
        /// Move tabs to the window containing this tab ID
        #[arg(long, value_name = "TAB_ID", conflicts_with = "to_window")]
        to_tab: Option<String>,
        /// Target index in the destination window (`-1` appends)
        #[arg(long, default_value_t = -1)]
        index: i64,
        /// Tab IDs to move; reads from stdin if none given
        tab_ids: Vec<String>,
    },
    /// Activate (focus) a tab by ID
    Activate {
        /// Tab ID (`prefix.pid.id`, or legacy `prefix.id`)
        tab_id: String,
    },
    /// Open a URL in a new tab
    Open {
        /// URL to open
        url: String,
        /// Target browser (uses first responsive connected browser if not specified)
        #[arg(short, long)]
        browser: Option<String>,
        /// Target window ID (`prefix.pid.w.id`, `prefix.w.id`, or raw browser window ID)
        #[arg(long, value_name = "WINDOW_ID")]
        window: Option<String>,
        /// Target index in the destination window
        #[arg(long)]
        index: Option<i64>,
    },
    /// Show connected browsers
    Clients,
    /// List read-only synced tabs discovered from local browser state
    Synced {
        #[command(subcommand)]
        command: SyncedCommand,
    },
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

#[derive(Subcommand)]
enum SyncedCommand {
    /// List synced tabs
    List {
        /// Output format
        #[arg(short, long, default_value = "tsv")]
        format: OutputFormat,
        /// Filter by browser (currently: orion)
        #[arg(short, long)]
        browser: Option<String>,
        /// Read the newest non-empty archived sync snapshot instead of current state
        #[arg(long)]
        archived: bool,
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
        Command::Windows { format, browser } => cmd_windows(&format, browser.as_deref()).await,
        Command::Close { tab_ids } => cmd_close(tab_ids).await,
        Command::Move {
            to_window,
            to_tab,
            index,
            tab_ids,
        } => cmd_move(tab_ids, to_window.as_deref(), to_tab.as_deref(), index).await,
        Command::Activate { tab_id } => cmd_activate(&tab_id).await,
        Command::Open {
            url,
            browser,
            window,
            index,
        } => cmd_open(&url, browser.as_deref(), window.as_deref(), index).await,
        Command::Clients => cmd_clients(),
        Command::Synced { command } => match command {
            SyncedCommand::List {
                format,
                browser,
                archived,
            } => cmd_synced_list(&format, browser.as_deref(), archived),
        },
        Command::Install {
            mediator_path,
            chrome_extension_id,
        } => cmd_install(mediator_path, chrome_extension_id),
    };

    std::process::exit(code);
}

// --- Socket discovery ---

#[derive(Clone, Debug, Eq, PartialEq)]
struct BrowserSocket {
    browser: String,
    pid: u32,
    path: PathBuf,
}

fn discover_sockets(browser_filter: Option<&str>) -> Vec<BrowserSocket> {
    let dir = socket_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };

    let mut sockets = entries
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
                pid,
                path: entry.path(),
            })
        })
        .collect::<Vec<_>>();

    sockets.sort_by(|left, right| {
        left.browser
            .cmp(&right.browser)
            .then(left.pid.cmp(&right.pid))
            .then(left.path.cmp(&right.path))
    });
    sockets
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

fn resolve_socket<'a>(
    sockets: &'a [BrowserSocket],
    prefix: &str,
    mediator_pid: Option<u32>,
) -> Result<&'a BrowserSocket, String> {
    let mut matching_sockets = sockets
        .iter()
        .filter(|socket| browser_prefix(&socket.browser) == prefix);

    if let Some(mediator_pid) = mediator_pid {
        return matching_sockets
            .find(|socket| socket.pid == mediator_pid)
            .ok_or_else(|| {
                format!(
                    "No browser connected for prefix '{}' with pid {}",
                    prefix, mediator_pid
                )
            });
    }

    let Some(first_match) = matching_sockets.next() else {
        return Err(format!("No browser connected for prefix '{}'", prefix));
    };

    if matching_sockets.next().is_some() {
        return Err(format!(
            "Multiple browsers connected for prefix '{}'. Use the full tab ID from `rustab list`.",
            prefix
        ));
    }

    Ok(first_match)
}

fn resolve_socket_for_tab_ref<'a>(
    sockets: &'a [BrowserSocket],
    tab_ref: TabRef<'_>,
) -> Result<&'a BrowserSocket, String> {
    resolve_socket(sockets, tab_ref.prefix, tab_ref.mediator_pid)
}

fn resolve_socket_for_window_ref<'a>(
    sockets: &'a [BrowserSocket],
    window_ref: WindowRef<'_>,
) -> Result<&'a BrowserSocket, String> {
    resolve_socket(sockets, window_ref.prefix, window_ref.mediator_pid)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TabListing {
    socket: BrowserSocket,
    tab_id: u64,
    window_id: u64,
    index: i64,
    title: String,
    url: String,
    active: bool,
    pinned: bool,
}

impl TabListing {
    fn from_response(socket: &BrowserSocket, tab: &Value) -> Self {
        Self {
            socket: socket.clone(),
            tab_id: tab.get("id").and_then(|value| value.as_u64()).unwrap_or(0),
            window_id: tab
                .get("window_id")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            index: tab
                .get("index")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
            title: tab
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            url: tab
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            active: tab
                .get("active")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            pinned: tab
                .get("pinned")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        }
    }

    fn display_id(&self) -> String {
        format_tab_id(
            browser_prefix(&self.socket.browser),
            self.socket.pid,
            self.tab_id,
        )
    }

    fn display_window_id(&self) -> String {
        format_window_id(
            browser_prefix(&self.socket.browser),
            self.socket.pid,
            self.window_id,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WindowListing {
    socket: BrowserSocket,
    window_id: u64,
    focused: bool,
    window_type: String,
    state: String,
    incognito: bool,
    tab_count: u64,
    active_tab_id: Option<u64>,
    active_tab_title: String,
    active_tab_url: String,
}

impl WindowListing {
    fn from_response(socket: &BrowserSocket, window: &Value) -> Self {
        Self {
            socket: socket.clone(),
            window_id: window
                .get("id")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            focused: window
                .get("focused")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            window_type: window
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            state: window
                .get("state")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            incognito: window
                .get("incognito")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            tab_count: window
                .get("tab_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            active_tab_id: window.get("active_tab_id").and_then(|value| value.as_u64()),
            active_tab_title: window
                .get("active_tab_title")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            active_tab_url: window
                .get("active_tab_url")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
        }
    }

    fn display_id(&self) -> String {
        format_window_id(
            browser_prefix(&self.socket.browser),
            self.socket.pid,
            self.window_id,
        )
    }

    fn active_tab_display_id(&self) -> Option<String> {
        self.active_tab_id.map(|tab_id| {
            format_tab_id(
                browser_prefix(&self.socket.browser),
                self.socket.pid,
                tab_id,
            )
        })
    }
}

/// Collect tab IDs from args, or read from stdin (one per line, first
/// tab-delimited field — so `rustab list | rustab close` works).
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

#[derive(Debug, Clone, Copy)]
enum WindowArg<'a> {
    Raw(u64),
    Scoped(WindowRef<'a>),
}

fn parse_window_arg(value: &str) -> Result<WindowArg<'_>, String> {
    if let Ok(window_id) = value.parse::<u64>() {
        return Ok(WindowArg::Raw(window_id));
    }

    parse_window_id(value).map(WindowArg::Scoped).ok_or_else(|| {
        format!(
            "Invalid window ID format: {value} (expected prefix.pid.w.id, prefix.w.id, or raw browser window id)"
        )
    })
}

fn same_socket(left: &BrowserSocket, right: &BrowserSocket) -> bool {
    left.browser == right.browser && left.pid == right.pid && left.path == right.path
}

fn parse_tab_ids(tab_ids: &[String]) -> Result<Vec<TabRef<'_>>, String> {
    tab_ids
        .iter()
        .map(|id_str| {
            parse_tab_id(id_str).ok_or_else(|| {
                format!("Invalid tab ID format: {id_str} (expected prefix.pid.id, e.g. c.4242.123)")
            })
        })
        .collect()
}

fn socket_for_raw_window_id<'a>(
    sockets: &'a [BrowserSocket],
    browser_filter: Option<&str>,
) -> Result<&'a BrowserSocket, String> {
    match sockets {
        [] => match browser_filter {
            Some(browser) => Err(format!("No browsers connected matching '{browser}'")),
            None => Err("No browsers connected".to_string()),
        },
        [socket] => Ok(socket),
        _ => Err(
            "Raw window IDs are ambiguous across multiple browser instances. Use a scoped window ID from `rustab windows`."
                .to_string(),
        ),
    }
}

fn validate_move_index(index: i64) -> Result<(), String> {
    if index < -1 {
        return Err("move index must be -1 or greater".to_string());
    }
    Ok(())
}

fn validate_open_index(index: Option<i64>) -> Result<(), String> {
    if matches!(index, Some(index) if index < 0) {
        return Err("open index must be 0 or greater".to_string());
    }
    Ok(())
}

async fn tab_window_id(sock: &BrowserSocket, tab_id: u64) -> Result<u64, String> {
    let request = json!({"id": 1, "method": "list_tabs"});
    let response = send_request(&sock.path, &request).await?;

    if let Some(error) = response.get("error") {
        return Err(error
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string()));
    }

    let tabs = response
        .get("result")
        .and_then(|result| result.as_array())
        .ok_or_else(|| "invalid response".to_string())?;

    tabs.iter()
        .find(|tab| tab.get("id").and_then(|value| value.as_u64()) == Some(tab_id))
        .and_then(|tab| tab.get("window_id").and_then(|value| value.as_u64()))
        .ok_or_else(|| format!("target tab {tab_id} was not found"))
}

// --- Commands ---

async fn cmd_list(format: &OutputFormat, browser_filter: Option<&str>) -> i32 {
    let sockets = discover_sockets(browser_filter);

    if sockets.is_empty() {
        eprintln!("No browsers connected. Is the extension installed and rustab-mediator running?");
        return 1;
    }

    let mut all_tabs = Vec::new();
    let mut successful_responses = 0;

    for sock in &sockets {
        let request = json!({"id": 1, "method": "list_tabs"});
        match send_request(&sock.path, &request).await {
            Ok(response) => {
                if let Some(tabs) = response.get("result").and_then(|r| r.as_array()) {
                    successful_responses += 1;
                    for tab in tabs {
                        all_tabs.push(TabListing::from_response(sock, tab));
                    }
                } else if let Some(err) = response.get("error") {
                    eprintln!("{} (pid {}): {}", sock.browser, sock.pid, err);
                } else {
                    eprintln!("{} (pid {}): invalid response", sock.browser, sock.pid);
                }
            }
            Err(e) => eprintln!("{} (pid {}): {e}", sock.browser, sock.pid),
        }
    }

    if successful_responses == 0 {
        return 1;
    }

    // Group tabs by browser instance first, then by window within that browser.
    all_tabs.sort_by(|left, right| {
        left.socket
            .browser
            .cmp(&right.socket.browser)
            .then(left.socket.pid.cmp(&right.socket.pid))
            .then(left.window_id.cmp(&right.window_id))
            .then(left.index.cmp(&right.index))
            .then(left.tab_id.cmp(&right.tab_id))
    });

    match format {
        OutputFormat::Json => {
            let out: Vec<Value> = all_tabs
                .iter()
                .map(|tab| {
                    json!({
                        "id": tab.display_id(),
                        "browser": tab.socket.browser.as_str(),
                        "mediator_pid": tab.socket.pid,
                        "window": tab.display_window_id(),
                        "window_id": tab.window_id,
                        "index": tab.index,
                        "title": tab.title.as_str(),
                        "url": tab.url.as_str(),
                        "active": tab.active,
                        "pinned": tab.pinned,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        OutputFormat::Tsv => {
            for tab in &all_tabs {
                println!("{}\t{}\t{}", tab.display_id(), tab.title, tab.url);
            }
        }
    }

    0
}

async fn cmd_windows(format: &OutputFormat, browser_filter: Option<&str>) -> i32 {
    let sockets = discover_sockets(browser_filter);

    if sockets.is_empty() {
        eprintln!("No browsers connected. Is the extension installed and rustab-mediator running?");
        return 1;
    }

    let mut all_windows = Vec::new();
    let mut successful_responses = 0;

    for sock in &sockets {
        let request = json!({"id": 1, "method": "list_windows"});
        match send_request(&sock.path, &request).await {
            Ok(response) => {
                if let Some(windows) = response.get("result").and_then(|r| r.as_array()) {
                    successful_responses += 1;
                    for window in windows {
                        all_windows.push(WindowListing::from_response(sock, window));
                    }
                } else if let Some(err) = response.get("error") {
                    eprintln!("{} (pid {}): {}", sock.browser, sock.pid, err);
                } else {
                    eprintln!("{} (pid {}): invalid response", sock.browser, sock.pid);
                }
            }
            Err(e) => eprintln!("{} (pid {}): {e}", sock.browser, sock.pid),
        }
    }

    if successful_responses == 0 {
        return 1;
    }

    all_windows.sort_by(|left, right| {
        left.socket
            .browser
            .cmp(&right.socket.browser)
            .then(left.socket.pid.cmp(&right.socket.pid))
            .then(left.window_id.cmp(&right.window_id))
    });

    match format {
        OutputFormat::Json => {
            let out: Vec<Value> = all_windows
                .iter()
                .map(|window| {
                    json!({
                        "id": window.display_id(),
                        "browser": window.socket.browser.as_str(),
                        "mediator_pid": window.socket.pid,
                        "window_id": window.window_id,
                        "focused": window.focused,
                        "type": window.window_type.as_str(),
                        "state": window.state.as_str(),
                        "incognito": window.incognito,
                        "tab_count": window.tab_count,
                        "active_tab_id": window.active_tab_display_id(),
                        "active_tab_raw_id": window.active_tab_id,
                        "active_tab_title": window.active_tab_title.as_str(),
                        "active_tab_url": window.active_tab_url.as_str(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        OutputFormat::Tsv => {
            for window in &all_windows {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    window.display_id(),
                    window.tab_count,
                    window.focused,
                    window.active_tab_title,
                    window.active_tab_url
                );
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

    let tab_refs = match parse_tab_ids(&tab_ids) {
        Ok(tab_refs) => tab_refs,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };

    // Group tab IDs by browser prefix and mediator PID.
    let mut by_socket: std::collections::BTreeMap<(String, Option<u32>), Vec<u64>> =
        std::collections::BTreeMap::new();

    for tab_ref in tab_refs {
        by_socket
            .entry((tab_ref.prefix.to_string(), tab_ref.mediator_pid))
            .or_default()
            .push(tab_ref.tab_id);
    }

    let sockets = discover_sockets(None);
    let mut failed = false;

    for ((prefix, mediator_pid), ids) in &by_socket {
        let sock = match resolve_socket(&sockets, prefix, *mediator_pid) {
            Ok(sock) => sock,
            Err(err) => {
                eprintln!("{err}");
                failed = true;
                continue;
            }
        };

        let request = json!({"id": 1, "method": "close_tabs", "params": {"tab_ids": ids}});
        match send_request(&sock.path, &request).await {
            Ok(response) => {
                if let Some(err) = response.get("error") {
                    eprintln!("{} (pid {}): {}", sock.browser, sock.pid, err);
                    failed = true;
                }
            }
            Err(e) => {
                eprintln!("{} (pid {}): {e}", sock.browser, sock.pid);
                failed = true;
            }
        }
    }

    i32::from(failed)
}

async fn cmd_move(
    tab_ids: Vec<String>,
    to_window: Option<&str>,
    to_tab: Option<&str>,
    index: i64,
) -> i32 {
    if let Err(error) = validate_move_index(index) {
        eprintln!("{error}");
        return 1;
    }

    let tab_ids = collect_tab_ids(tab_ids);

    if tab_ids.is_empty() {
        eprintln!("No tab IDs provided. Pass as arguments or pipe from `rustab list`.");
        return 1;
    }

    let tab_refs = match parse_tab_ids(&tab_ids) {
        Ok(tab_refs) => tab_refs,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };

    let sockets = discover_sockets(None);
    let mut source_socket = None;
    let mut raw_tab_ids = Vec::with_capacity(tab_refs.len());

    for tab_ref in &tab_refs {
        let sock = match resolve_socket_for_tab_ref(&sockets, *tab_ref) {
            Ok(sock) => sock,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };

        if let Some(existing_socket) = source_socket {
            if !same_socket(existing_socket, sock) {
                eprintln!("Cannot move tabs across browser instances.");
                return 1;
            }
        } else {
            source_socket = Some(sock);
        }

        raw_tab_ids.push(tab_ref.tab_id);
    }

    let source_socket = source_socket.expect("tab_refs is non-empty");
    let target_window_id = match (to_window, to_tab) {
        (Some(window_id), None) => match parse_window_arg(window_id) {
            Ok(WindowArg::Raw(window_id)) => window_id,
            Ok(WindowArg::Scoped(window_ref)) => {
                let target_socket = match resolve_socket_for_window_ref(&sockets, window_ref) {
                    Ok(sock) => sock,
                    Err(error) => {
                        eprintln!("{error}");
                        return 1;
                    }
                };

                if !same_socket(source_socket, target_socket) {
                    eprintln!("Cannot move tabs across browser instances.");
                    return 1;
                }

                window_ref.window_id
            }
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        },
        (None, Some(tab_id)) => {
            let Some(target_tab_ref) = parse_tab_id(tab_id) else {
                eprintln!(
                    "Invalid tab ID format: {tab_id} (expected prefix.pid.id, e.g. c.4242.123)"
                );
                return 1;
            };

            let target_socket = match resolve_socket_for_tab_ref(&sockets, target_tab_ref) {
                Ok(sock) => sock,
                Err(error) => {
                    eprintln!("{error}");
                    return 1;
                }
            };

            if !same_socket(source_socket, target_socket) {
                eprintln!("Cannot move tabs across browser instances.");
                return 1;
            }

            match tab_window_id(target_socket, target_tab_ref.tab_id).await {
                Ok(window_id) => window_id,
                Err(error) => {
                    eprintln!(
                        "{} (pid {}): {error}",
                        target_socket.browser, target_socket.pid
                    );
                    return 1;
                }
            }
        }
        _ => {
            eprintln!("A target is required. Pass --to-window or --to-tab.");
            return 1;
        }
    };

    let request = json!({
        "id": 1,
        "method": "move_tabs",
        "params": {
            "tab_ids": raw_tab_ids,
            "window_id": target_window_id,
            "index": index,
        },
    });

    match send_request(&source_socket.path, &request).await {
        Ok(response) => {
            if let Some(err) = response.get("error") {
                eprintln!(
                    "{} (pid {}): {}",
                    source_socket.browser, source_socket.pid, err
                );
                return 1;
            }
            0
        }
        Err(error) => {
            eprintln!(
                "{} (pid {}): {error}",
                source_socket.browser, source_socket.pid
            );
            1
        }
    }
}

async fn cmd_activate(tab_id: &str) -> i32 {
    let Some(tab_ref) = parse_tab_id(tab_id) else {
        eprintln!("Invalid tab ID format: {tab_id} (expected prefix.pid.id, e.g. c.4242.123)");
        return 1;
    };

    let sockets = discover_sockets(None);
    let sock = match resolve_socket_for_tab_ref(&sockets, tab_ref) {
        Ok(sock) => sock,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let request = json!({"id": 1, "method": "activate_tab", "params": {"tab_id": tab_ref.tab_id}});
    match send_request(&sock.path, &request).await {
        Ok(response) => {
            if let Some(err) = response.get("error") {
                eprintln!("{} (pid {}): {}", sock.browser, sock.pid, err);
                return 1;
            }
            0
        }
        Err(e) => {
            eprintln!("{} (pid {}): {e}", sock.browser, sock.pid);
            1
        }
    }
}

async fn cmd_open(
    url: &str,
    browser_filter: Option<&str>,
    window: Option<&str>,
    index: Option<i64>,
) -> i32 {
    if let Err(error) = validate_open_index(index) {
        eprintln!("{error}");
        return 1;
    }

    let sockets = discover_sockets(browser_filter);

    if sockets.is_empty() {
        match browser_filter {
            Some(browser) => eprintln!("No browsers connected matching '{browser}'"),
            None => eprintln!("No browsers connected"),
        }
        return 1;
    }

    let (target_sockets, window_id) = match window {
        Some(window) => match parse_window_arg(window) {
            Ok(WindowArg::Raw(window_id)) => {
                let sock = match socket_for_raw_window_id(&sockets, browser_filter) {
                    Ok(sock) => sock,
                    Err(error) => {
                        eprintln!("{error}");
                        return 1;
                    }
                };
                (vec![sock], Some(window_id))
            }
            Ok(WindowArg::Scoped(window_ref)) => {
                let sock = match resolve_socket_for_window_ref(&sockets, window_ref) {
                    Ok(sock) => sock,
                    Err(error) => {
                        eprintln!("{error}");
                        return 1;
                    }
                };
                (vec![sock], Some(window_ref.window_id))
            }
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        },
        None => (sockets.iter().collect::<Vec<_>>(), None),
    };

    let mut params = serde_json::Map::new();
    params.insert("url".to_string(), json!(url));
    if let Some(window_id) = window_id {
        params.insert("window_id".to_string(), json!(window_id));
    }
    if let Some(index) = index {
        params.insert("index".to_string(), json!(index));
    }

    let request = json!({"id": 1, "method": "open_tab", "params": Value::Object(params)});
    let mut errors = Vec::new();

    for sock in target_sockets {
        match send_request(&sock.path, &request).await {
            Ok(response) => {
                if let Some(err) = response.get("error") {
                    errors.push(format!("{} (pid {}): {}", sock.browser, sock.pid, err));
                    continue;
                }

                let Some(result) = response.get("result") else {
                    errors.push(format!(
                        "{} (pid {}): invalid response",
                        sock.browser, sock.pid
                    ));
                    continue;
                };
                let Some(tab_id) = result.get("id").and_then(|value| value.as_u64()) else {
                    errors.push(format!(
                        "{} (pid {}): missing tab id in response",
                        sock.browser, sock.pid
                    ));
                    continue;
                };

                println!(
                    "{}",
                    format_tab_id(browser_prefix(&sock.browser), sock.pid, tab_id)
                );
                return 0;
            }
            Err(e) => errors.push(format!("{} (pid {}): {e}", sock.browser, sock.pid)),
        }
    }

    for error in errors {
        eprintln!("{error}");
    }

    1
}

fn cmd_clients() -> i32 {
    let sockets = discover_sockets(None);

    if sockets.is_empty() {
        eprintln!("No browsers connected.");
        return 1;
    }

    for sock in &sockets {
        let prefix = browser_prefix(&sock.browser);
        println!(
            "{prefix}\t{}\t{}\t{}",
            sock.browser,
            sock.pid,
            sock.path.display()
        );
    }

    0
}

fn cmd_synced_list(format: &OutputFormat, browser_filter: Option<&str>, archived: bool) -> i32 {
    let mut tabs = match synced::list_synced_tabs(browser_filter, archived) {
        Ok(tabs) => tabs,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };

    tabs.sort_by(|left, right| {
        right
            .browser
            .cmp(&left.browser)
            .then_with(|| right.last_synced.cmp(&left.last_synced))
            .then_with(|| left.title.cmp(&right.title))
    });

    match format {
        OutputFormat::Json => {
            let out: Vec<Value> = tabs.iter().map(synced_tab_json).collect();
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        OutputFormat::Tsv => {
            for tab in &tabs {
                let device_id = tab.device_id.as_deref().unwrap_or("");
                let last_synced = tab.last_synced.as_deref().unwrap_or("");
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    tab.id, tab.source, device_id, last_synced, tab.title, tab.url
                );
            }
        }
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

    let mut installed_locations = 0;
    let mut installed_browsers = 0;

    for browser in BROWSERS {
        let config_path = PathBuf::from(&home).join(browser.config_dir);
        if !config_path.exists() {
            continue;
        }

        let manifest = if browser.is_firefox {
            build_firefox_manifest(&mediator_abs)
        } else {
            build_chrome_manifest(&mediator_abs, &chrome_ext_id)
        };

        let mut wrote_manifest_for_browser = false;

        for manifest_dir in manifest_target_dirs(Path::new(&home), browser) {
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

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

fn manifest_target_dirs(
    home: &Path,
    browser: &rustab_protocol::BrowserManifestInfo,
) -> Vec<PathBuf> {
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

fn synced_tab_json(tab: &SyncedTab) -> Value {
    json!({
        "id": tab.id,
        "browser": tab.browser,
        "kind": "synced",
        "source": tab.source,
        "device_id": tab.device_id,
        "window_name": tab.window_name,
        "window_id": tab.window_id,
        "title": tab.title,
        "url": tab.url,
        "pinned": tab.pinned,
        "last_synced": tab.last_synced,
        "modified": tab.modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket(browser: &str, pid: u32) -> BrowserSocket {
        BrowserSocket {
            browser: browser.to_string(),
            pid,
            path: PathBuf::from(format!("/tmp/{browser}-{pid}.sock")),
        }
    }

    #[test]
    fn resolves_legacy_tab_ids_when_only_one_socket_matches() {
        let sockets = vec![socket("brave", 101)];

        let resolved = resolve_socket_for_tab_ref(
            &sockets,
            TabRef {
                prefix: "b",
                mediator_pid: None,
                tab_id: 42,
            },
        )
        .expect("single Brave socket should resolve");

        assert_eq!(resolved.pid, 101);
    }

    #[test]
    fn rejects_legacy_tab_ids_when_multiple_sockets_match() {
        let sockets = vec![socket("brave", 101), socket("brave", 202)];

        let err = resolve_socket_for_tab_ref(
            &sockets,
            TabRef {
                prefix: "b",
                mediator_pid: None,
                tab_id: 42,
            },
        )
        .expect_err("legacy IDs should be ambiguous across multiple Brave sockets");

        assert!(err.contains("Multiple browsers connected"));
    }

    #[test]
    fn resolves_full_tab_ids_to_the_matching_socket() {
        let sockets = vec![socket("brave", 101), socket("brave", 202)];

        let resolved = resolve_socket_for_tab_ref(
            &sockets,
            TabRef {
                prefix: "b",
                mediator_pid: Some(202),
                tab_id: 42,
            },
        )
        .expect("full IDs should resolve to a specific socket");

        assert_eq!(resolved.pid, 202);
    }

    #[test]
    fn resolves_full_window_ids_to_the_matching_socket() {
        let sockets = vec![socket("brave", 101), socket("brave", 202)];

        let resolved = resolve_socket_for_window_ref(
            &sockets,
            WindowRef {
                prefix: "b",
                mediator_pid: Some(202),
                window_id: 42,
            },
        )
        .expect("full window IDs should resolve to a specific socket");

        assert_eq!(resolved.pid, 202);
    }

    #[test]
    fn raw_window_ids_require_a_single_candidate_socket() {
        let sockets = vec![socket("brave", 101), socket("brave", 202)];

        let err = socket_for_raw_window_id(&sockets, None)
            .expect_err("raw window IDs should be ambiguous with multiple candidates");

        assert!(err.contains("Raw window IDs are ambiguous"));
    }

    #[test]
    fn raw_window_ids_use_the_only_candidate_socket() {
        let sockets = vec![socket("brave", 101)];

        let resolved = socket_for_raw_window_id(&sockets, None)
            .expect("raw window IDs should work with one candidate");

        assert_eq!(resolved.pid, 101);
    }

    #[test]
    fn validates_move_index() {
        assert!(validate_move_index(-1).is_ok());
        assert!(validate_move_index(0).is_ok());
        assert!(validate_move_index(-2).is_err());
    }

    #[test]
    fn validates_open_index() {
        assert!(validate_open_index(None).is_ok());
        assert!(validate_open_index(Some(0)).is_ok());
        assert!(validate_open_index(Some(-1)).is_err());
    }

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
