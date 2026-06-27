use crate::client::{send_rpc, BrowserSocket};
use crate::output::write_tsv_row;
use rustab_protocol::{
    browser_prefix, format_tab_id, format_window_id, RpcRequest, TabInfo, WindowInfo,
    LIST_TABS_METHOD, LIST_WINDOWS_METHOD,
};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::io;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TabListing {
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
    pub(crate) fn new(socket: &BrowserSocket, tab: TabInfo) -> Self {
        Self {
            socket: socket.clone(),
            tab_id: tab.id,
            window_id: tab.window_id,
            index: tab.index,
            title: tab.title,
            url: tab.url,
            active: tab.active,
            pinned: tab.pinned,
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

    fn sort_key_cmp(&self, other: &Self) -> Ordering {
        self.socket
            .cmp(&other.socket)
            .then(self.window_id.cmp(&other.window_id))
            .then(self.index.cmp(&other.index))
            .then(self.tab_id.cmp(&other.tab_id))
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.display_id(),
            "browser": self.socket.browser.as_str(),
            "mediator_pid": self.socket.pid,
            "window": self.display_window_id(),
            "window_id": self.window_id,
            "index": self.index,
            "title": self.title.as_str(),
            "url": self.url.as_str(),
            "active": self.active,
            "pinned": self.pinned,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowListing {
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
    pub(crate) fn new(socket: &BrowserSocket, window: WindowInfo) -> Self {
        Self {
            socket: socket.clone(),
            window_id: window.id,
            focused: window.focused,
            window_type: window.window_type,
            state: window.state,
            incognito: window.incognito,
            tab_count: window.tab_count,
            active_tab_id: window.active_tab_id,
            active_tab_title: window.active_tab_title,
            active_tab_url: window.active_tab_url,
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

    fn sort_key_cmp(&self, other: &Self) -> Ordering {
        self.socket
            .cmp(&other.socket)
            .then(self.window_id.cmp(&other.window_id))
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.display_id(),
            "browser": self.socket.browser.as_str(),
            "mediator_pid": self.socket.pid,
            "window_id": self.window_id,
            "focused": self.focused,
            "type": self.window_type.as_str(),
            "state": self.state.as_str(),
            "incognito": self.incognito,
            "tab_count": self.tab_count,
            "active_tab_id": self.active_tab_display_id(),
            "active_tab_raw_id": self.active_tab_id,
            "active_tab_title": self.active_tab_title.as_str(),
            "active_tab_url": self.active_tab_url.as_str(),
        })
    }
}

pub(crate) fn sort_tab_listings(tabs: &mut [TabListing]) {
    tabs.sort_by(TabListing::sort_key_cmp);
}

pub(crate) fn sort_window_listings(windows: &mut [WindowListing]) {
    windows.sort_by(WindowListing::sort_key_cmp);
}

/// Fetch tab listings from connected browser mediators.
///
/// Returns `None` if every socket failed (errors are printed to stderr).
pub(crate) async fn fetch_tab_listings(sockets: &[BrowserSocket]) -> Option<Vec<TabListing>> {
    let mut tabs = Vec::new();
    let mut successful_responses = 0;

    for sock in sockets {
        let request = RpcRequest::new(LIST_TABS_METHOD, json!({}));
        match send_rpc::<Vec<TabInfo>>(sock, &request).await {
            Ok(items) => {
                successful_responses += 1;
                tabs.extend(items.into_iter().map(|tab| TabListing::new(sock, tab)));
            }
            Err(error) => eprintln!("{} (pid {}): {error}", sock.browser, sock.pid),
        }
    }

    (successful_responses > 0).then_some(tabs)
}

/// Fetch window listings from connected browser mediators.
///
/// Returns `None` if every socket failed (errors are printed to stderr).
pub(crate) async fn fetch_window_listings(sockets: &[BrowserSocket]) -> Option<Vec<WindowListing>> {
    let mut windows = Vec::new();
    let mut successful_responses = 0;

    for sock in sockets {
        let request = RpcRequest::new(LIST_WINDOWS_METHOD, json!({}));
        let result = send_rpc::<Vec<WindowInfo>>(sock, &request)
            .await
            .map(|items| {
                items
                    .into_iter()
                    .map(|w| WindowListing::new(sock, w))
                    .collect::<Vec<_>>()
            });

        match result {
            Ok(mut rows) => {
                successful_responses += 1;
                windows.append(&mut rows);
            }
            Err(error) => eprintln!("{} (pid {}): {error}", sock.browser, sock.pid),
        }
    }

    (successful_responses > 0).then_some(windows)
}

pub(crate) fn tab_listings_json(tabs: &[TabListing]) -> Vec<Value> {
    tabs.iter().map(TabListing::to_json).collect()
}

pub(crate) fn window_listings_json(windows: &[WindowListing]) -> Vec<Value> {
    windows.iter().map(WindowListing::to_json).collect()
}

pub(crate) fn print_tab_listings_tsv(tabs: &[TabListing]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    for tab in tabs {
        let display_id = tab.display_id();
        write_tsv_row(
            &mut output,
            [display_id.as_str(), tab.title.as_str(), tab.url.as_str()],
        )?;
    }
    Ok(())
}

pub(crate) fn print_window_listings_tsv(windows: &[WindowListing]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    for window in windows {
        let display_id = window.display_id();
        let tab_count = window.tab_count.to_string();
        let focused = window.focused.to_string();
        write_tsv_row(
            &mut output,
            [
                display_id.as_str(),
                tab_count.as_str(),
                focused.as_str(),
                window.active_tab_title.as_str(),
                window.active_tab_url.as_str(),
            ],
        )?;
    }
    Ok(())
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

    fn tab_listing(socket: &BrowserSocket, id: u64, window_id: u64, index: i64) -> TabListing {
        TabListing::new(
            socket,
            TabInfo {
                id,
                title: format!("tab {id}"),
                url: format!("https://example.test/{id}"),
                active: false,
                window_id,
                index,
                pinned: false,
            },
        )
    }

    #[test]
    fn sorts_tabs_by_socket_then_window_index_and_tab_id() {
        let brave = socket("brave", 7);
        let firefox = socket("firefox", 3);
        let mut tabs = vec![
            tab_listing(&firefox, 1, 1, 0),
            tab_listing(&brave, 10, 2, 1),
            tab_listing(&brave, 9, 2, 0),
        ];

        sort_tab_listings(&mut tabs);

        let display_ids = tabs.iter().map(TabListing::display_id).collect::<Vec<_>>();
        assert_eq!(display_ids, ["b.7.9", "b.7.10", "f.3.1"]);
    }

    #[test]
    fn serializes_window_listing_json_shape() {
        let window = WindowListing::new(
            &socket("brave", 7),
            WindowInfo {
                id: 42,
                focused: true,
                window_type: "normal".to_string(),
                state: "maximized".to_string(),
                incognito: false,
                tab_count: 3,
                active_tab_id: Some(9),
                active_tab_title: "active".to_string(),
                active_tab_url: "https://example.test/active".to_string(),
            },
        );

        assert_eq!(
            window_listings_json(&[window]),
            vec![json!({
                "id": "b.7.w.42",
                "browser": "brave",
                "mediator_pid": 7,
                "window_id": 42,
                "focused": true,
                "type": "normal",
                "state": "maximized",
                "incognito": false,
                "tab_count": 3,
                "active_tab_id": "b.7.9",
                "active_tab_raw_id": 9,
                "active_tab_title": "active",
                "active_tab_url": "https://example.test/active",
            })]
        );
    }
}
