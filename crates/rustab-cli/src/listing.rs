use crate::client::BrowserSocket;
use rustab_protocol::{browser_prefix, format_tab_id, format_window_id, TabInfo, WindowInfo};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TabListing {
    pub socket: BrowserSocket,
    pub tab_id: u64,
    pub window_id: u64,
    pub index: i64,
    pub title: String,
    pub url: String,
    pub active: bool,
    pub pinned: bool,
}

impl TabListing {
    pub fn new(socket: &BrowserSocket, tab: TabInfo) -> Self {
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

    pub fn display_id(&self) -> String {
        format_tab_id(
            browser_prefix(&self.socket.browser),
            self.socket.pid,
            self.tab_id,
        )
    }

    pub fn display_window_id(&self) -> String {
        format_window_id(
            browser_prefix(&self.socket.browser),
            self.socket.pid,
            self.window_id,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowListing {
    pub socket: BrowserSocket,
    pub window_id: u64,
    pub focused: bool,
    pub window_type: String,
    pub state: String,
    pub incognito: bool,
    pub tab_count: u64,
    pub active_tab_id: Option<u64>,
    pub active_tab_title: String,
    pub active_tab_url: String,
}

impl WindowListing {
    pub fn new(socket: &BrowserSocket, window: WindowInfo) -> Self {
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

    pub fn display_id(&self) -> String {
        format_window_id(
            browser_prefix(&self.socket.browser),
            self.socket.pid,
            self.window_id,
        )
    }

    pub fn active_tab_display_id(&self) -> Option<String> {
        self.active_tab_id.map(|tab_id| {
            format_tab_id(
                browser_prefix(&self.socket.browser),
                self.socket.pid,
                tab_id,
            )
        })
    }
}
