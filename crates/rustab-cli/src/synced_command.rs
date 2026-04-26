use crate::cli::OutputFormat;
use crate::output::print_json;
use crate::synced::{self, SyncedTab};
use serde_json::{json, Value};

pub fn cmd_synced_list(format: &OutputFormat, browser_filter: Option<&str>, archived: bool) -> i32 {
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
            if let Err(error) = print_json(&out) {
                eprintln!("{error}");
                return 1;
            }
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
