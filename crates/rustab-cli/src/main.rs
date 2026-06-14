mod cli;
mod client;
mod doctor;
mod input;
mod install;
mod listing;
mod output;
mod synced;
mod synced_command;

use crate::cli::{Cli, Command, OutputFormat, SyncedCommand};
use crate::client::{
    discover_sockets, resolve_socket, resolve_socket_for_tab_ref, resolve_socket_for_window_ref,
    send_rpc, socket_for_raw_window_id, BrowserSocket,
};
use crate::doctor::cmd_doctor;
use crate::input::{
    collect_tab_ids, parse_tab_ids, parse_window_arg, validate_move_index, validate_open_index,
    WindowArg,
};
use crate::install::cmd_install;
use crate::output::print_json;
use crate::synced_command::cmd_synced_list;
use clap::Parser;
use rustab_protocol::{
    browser_prefix, format_tab_id, parse_tab_id, RpcRequest, TabInfo, ACTIVATE_TAB_METHOD,
    CLOSE_TABS_METHOD, LIST_TABS_METHOD, MOVE_TABS_METHOD, OPEN_TAB_METHOD,
};
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    reset_sigpipe();

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
        Command::Doctor { browser } => cmd_doctor(browser.as_deref()).await,
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

fn reset_sigpipe() {
    // Rust ignores SIGPIPE by default. Resetting it lets `rustab list | head`
    // exit quietly instead of panicking when the downstream reader closes.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

async fn tab_window_id(sock: &BrowserSocket, tab_id: u64) -> Result<u64, String> {
    let tabs: Vec<TabInfo> = send_rpc(sock, &RpcRequest::new(LIST_TABS_METHOD, json!({}))).await?;

    tabs.into_iter()
        .find(|tab| tab.id == tab_id)
        .map(|tab| tab.window_id)
        .ok_or_else(|| format!("target tab {tab_id} was not found"))
}

async fn cmd_list(format: &OutputFormat, browser_filter: Option<&str>) -> i32 {
    let sockets = discover_sockets(browser_filter);

    if sockets.is_empty() {
        eprintln!("No browsers connected. Is the extension installed and rustab-mediator running?");
        return 1;
    }

    let Some(mut all_tabs) = listing::fetch_tab_listings(&sockets).await else {
        return 1;
    };
    listing::sort_tab_listings(&mut all_tabs);

    match format {
        OutputFormat::Json => {
            if let Err(error) = print_json(&listing::tab_listings_json(&all_tabs)) {
                eprintln!("{error}");
                return 1;
            }
        }
        OutputFormat::Tsv => listing::print_tab_listings_tsv(&all_tabs),
    }

    0
}

async fn cmd_windows(format: &OutputFormat, browser_filter: Option<&str>) -> i32 {
    let sockets = discover_sockets(browser_filter);

    if sockets.is_empty() {
        eprintln!("No browsers connected. Is the extension installed and rustab-mediator running?");
        return 1;
    }

    let Some(mut all_windows) = listing::fetch_window_listings(&sockets).await else {
        return 1;
    };

    listing::sort_window_listings(&mut all_windows);

    match format {
        OutputFormat::Json => {
            if let Err(error) = print_json(&listing::window_listings_json(&all_windows)) {
                eprintln!("{error}");
                return 1;
            }
        }
        OutputFormat::Tsv => listing::print_window_listings_tsv(&all_windows),
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

        let request = RpcRequest::new(CLOSE_TABS_METHOD, json!({"tab_ids": ids}));
        if let Err(error) = send_rpc::<Value>(sock, &request).await {
            eprintln!("{} (pid {}): {error}", sock.browser, sock.pid);
            failed = true;
        }
    }

    i32::from(failed)
}

fn require_same_browser_instance(
    source: &BrowserSocket,
    target: &BrowserSocket,
) -> Result<(), &'static str> {
    if source == target {
        Ok(())
    } else {
        Err("Cannot move tabs across browser instances.")
    }
}

/// Resolve the target window ID for a move operation from `--to-window` or `--to-tab`.
async fn resolve_target_window_id(
    sockets: &[BrowserSocket],
    source_socket: &BrowserSocket,
    to_window: Option<&str>,
    to_tab: Option<&str>,
) -> Result<u64, String> {
    match (to_window, to_tab) {
        (Some(window_arg), _) => match parse_window_arg(window_arg)? {
            WindowArg::Raw(window_id) => Ok(window_id),
            WindowArg::Scoped(window_ref) => {
                let target_socket = resolve_socket_for_window_ref(sockets, window_ref)
                    .map_err(|e| e.to_string())?;
                require_same_browser_instance(source_socket, target_socket)
                    .map_err(|e| e.to_string())?;
                Ok(window_ref.window_id)
            }
        },
        (_, Some(tab_id)) => {
            let target_tab_ref = parse_tab_id(tab_id).ok_or_else(|| {
                format!("Invalid tab ID format: {tab_id} (expected prefix.pid.id, e.g. c.4242.123)")
            })?;
            let target_socket =
                resolve_socket_for_tab_ref(sockets, target_tab_ref).map_err(|e| e.to_string())?;
            require_same_browser_instance(source_socket, target_socket)
                .map_err(|e| e.to_string())?;
            tab_window_id(target_socket, target_tab_ref.tab_id)
                .await
                .map_err(|error| {
                    format!(
                        "{} (pid {}): {error}",
                        target_socket.browser, target_socket.pid
                    )
                })
        }
        _ => Err("A target is required. Pass --to-window or --to-tab.".to_string()),
    }
}

/// Resolve a window argument to (socket, window_id).
fn resolve_window_socket<'a>(
    sockets: &'a [BrowserSocket],
    window_arg: &str,
) -> Result<(&'a BrowserSocket, u64), String> {
    match parse_window_arg(window_arg)? {
        WindowArg::Raw(window_id) => {
            socket_for_raw_window_id(sockets, None).map(|sock| (sock, window_id))
        }
        WindowArg::Scoped(window_ref) => resolve_socket_for_window_ref(sockets, window_ref)
            .map(|sock| (sock, window_ref.window_id)),
    }
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
            if let Err(error) = require_same_browser_instance(existing_socket, sock) {
                eprintln!("{error}");
                return 1;
            }
        } else {
            source_socket = Some(sock);
        }

        raw_tab_ids.push(tab_ref.tab_id);
    }

    let Some(source_socket) = source_socket else {
        eprintln!("No tab IDs provided. Pass as arguments or pipe from `rustab list`.");
        return 1;
    };

    let target_window_id =
        match resolve_target_window_id(&sockets, source_socket, to_window, to_tab).await {
            Ok(id) => id,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };

    let request = RpcRequest::new(
        MOVE_TABS_METHOD,
        json!({
            "tab_ids": raw_tab_ids,
            "window_id": target_window_id,
            "index": index,
        }),
    );

    match send_rpc::<Value>(source_socket, &request).await {
        Ok(_) => 0,
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

    let request = RpcRequest::new(ACTIVATE_TAB_METHOD, json!({"tab_id": tab_ref.tab_id}));
    match send_rpc::<Value>(sock, &request).await {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("{} (pid {}): {error}", sock.browser, sock.pid);
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
        Some(window_arg) => {
            let (sock, id) = match resolve_window_socket(&sockets, window_arg) {
                Ok(pair) => pair,
                Err(error) => {
                    eprintln!("{error}");
                    return 1;
                }
            };
            (vec![sock], Some(id))
        }
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

    let request = RpcRequest::new(OPEN_TAB_METHOD, Value::Object(params));
    let mut errors = Vec::new();

    for sock in target_sockets {
        match send_rpc::<TabInfo>(sock, &request).await {
            Ok(tab) => {
                println!(
                    "{}",
                    format_tab_id(browser_prefix(&sock.browser), sock.pid, tab.id)
                );
                return 0;
            }
            Err(error) => errors.push(format!("{} (pid {}): {error}", sock.browser, sock.pid)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn socket(browser: &str, pid: u32) -> BrowserSocket {
        BrowserSocket {
            browser: browser.to_string(),
            pid,
            path: PathBuf::from(format!("/tmp/{browser}-{pid}.sock")),
        }
    }

    #[tokio::test]
    async fn raw_move_target_uses_source_socket_without_global_disambiguation() {
        let source_socket = socket("brave", 7);
        let other_socket = socket("firefox", 9);
        let sockets = vec![source_socket.clone(), other_socket];

        let window_id = resolve_target_window_id(&sockets, &source_socket, Some("7"), None)
            .await
            .expect("raw window target should resolve within the source socket");

        assert_eq!(window_id, 7);
    }
}
