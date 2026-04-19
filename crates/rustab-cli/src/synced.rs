use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedTab {
    pub id: String,
    pub browser: String,
    pub source: String,
    pub device_id: Option<String>,
    pub window_name: Option<String>,
    pub window_id: Option<String>,
    pub title: String,
    pub url: String,
    pub pinned: bool,
    pub last_synced: Option<String>,
    pub modified: Option<String>,
}

pub fn list_synced_tabs(
    browser_filter: Option<&str>,
    archived: bool,
) -> Result<Vec<SyncedTab>, String> {
    let home = std::env::var("HOME").map_err(|_| "$HOME not set".to_string())?;
    list_synced_tabs_from_home(Path::new(&home), browser_filter, archived)
}

fn list_synced_tabs_from_home(
    home: &Path,
    browser_filter: Option<&str>,
    archived: bool,
) -> Result<Vec<SyncedTab>, String> {
    match browser_filter {
        Some("orion") | None => {
            let mut tabs = list_orion_synced_tabs(home, archived)?;
            if let Some(filter) = browser_filter {
                tabs.retain(|tab| tab.browser == filter);
            }
            Ok(tabs)
        }
        Some(other) => Err(format!(
            "Synced tabs are not supported for browser '{other}' yet"
        )),
    }
}

fn list_orion_synced_tabs(home: &Path, archived: bool) -> Result<Vec<SyncedTab>, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (home, archived);
        return Ok(vec![]);
    }

    #[cfg(target_os = "macos")]
    {
        let defaults_dir = home.join("Library/Application Support/Orion/Defaults");
        let current_path = defaults_dir.join(".local_named_windows.plist");

        if !archived {
            if !current_path.is_file() {
                return Ok(vec![]);
            }

            return parse_orion_synced_snapshot(&current_path, "current");
        }

        latest_non_empty_orion_snapshot(&defaults_dir)
            .map(|result| result.map(|(_, tabs)| tabs).unwrap_or_default())
    }
}

#[cfg(target_os = "macos")]
fn latest_non_empty_orion_snapshot(
    defaults_dir: &Path,
) -> Result<Option<(PathBuf, Vec<SyncedTab>)>, String> {
    let Ok(entries) = std::fs::read_dir(defaults_dir) else {
        return Ok(None);
    };

    let mut snapshots: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            if !name.starts_with("bk_") {
                return None;
            }

            let path = entry.path().join(".local_named_windows.plist");
            if !path.is_file() {
                return None;
            }

            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            Some((modified, path))
        })
        .collect();

    snapshots.sort_by(|(left_time, left_path), (right_time, right_path)| {
        right_time
            .cmp(left_time)
            .then_with(|| right_path.cmp(left_path))
    });

    for (_, snapshot) in snapshots {
        let tabs = parse_orion_synced_snapshot(&snapshot, "archived")?;
        if !tabs.is_empty() {
            return Ok(Some((snapshot, tabs)));
        }
    }

    Ok(None)
}

#[cfg(target_os = "macos")]
fn parse_orion_synced_snapshot(path: &Path, source: &str) -> Result<Vec<SyncedTab>, String> {
    let value = Value::from_file(path).map_err(|e| {
        format!(
            "failed to read Orion synced tabs from {}: {e}",
            path.display()
        )
    })?;
    let windows = value
        .as_array()
        .ok_or_else(|| format!("unexpected Orion synced tabs format in {}", path.display()))?;

    let mut tabs = Vec::new();

    for window in windows {
        let Some(window_dict) = window.as_dictionary() else {
            continue;
        };

        let window_info = dict_at(window_dict, "windowInfo");
        let window_name = window_info.and_then(|info| string_at(info, "name"));
        let window_id = window_info.and_then(|info| string_at(info, "namedWindowID"));

        let Some(tab_values) = array_at(window_dict, "tabsInfo") else {
            continue;
        };

        for (index, tab_value) in tab_values.iter().enumerate() {
            let Some(tab) = tab_value.as_dictionary() else {
                continue;
            };

            let title = string_at(tab, "title").unwrap_or_default();
            let url = string_at(tab, "url").unwrap_or_default();
            if title.is_empty() && url.is_empty() {
                continue;
            }

            let id = string_at(tab, "iCloudTabIdentifier")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("orion-sync-{}", index + 1));

            tabs.push(SyncedTab {
                id,
                browser: "orion".to_string(),
                source: source.to_string(),
                device_id: string_at(tab, "lastDeviceID"),
                window_name: window_name.clone(),
                window_id: window_id.clone(),
                title,
                url,
                pinned: bool_at(tab, "isPinned").unwrap_or(false),
                last_synced: date_string_at(tab, "lastSynced"),
                modified: date_string_at(tab, "modified"),
            });
        }
    }

    Ok(tabs)
}

fn dict_at<'a>(dict: &'a Dictionary, key: &str) -> Option<&'a Dictionary> {
    dict.get(key)?.as_dictionary()
}

fn array_at<'a>(dict: &'a Dictionary, key: &str) -> Option<&'a [Value]> {
    Some(dict.get(key)?.as_array()?.as_slice())
}

fn bool_at(dict: &Dictionary, key: &str) -> Option<bool> {
    dict.get(key)?.as_boolean()
}

fn string_at(dict: &Dictionary, key: &str) -> Option<String> {
    match dict.get(key)? {
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn date_string_at(dict: &Dictionary, key: &str) -> Option<String> {
    match dict.get(key)? {
        Value::Date(value) => Some(value.to_xml_format()),
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const ORION_SYNCED_TABS_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
  <dict>
    <key>windowInfo</key>
    <dict>
      <key>name</key>
      <string>Window 1</string>
      <key>namedWindowID</key>
      <string>B9E48D83-2884-4362-A4FD-A0747EFA6CBB</string>
    </dict>
    <key>tabsInfo</key>
    <array>
      <dict>
        <key>iCloudTabIdentifier</key>
        <string>321B9D93-048B-466C-8067-66129EB2A40E</string>
        <key>lastDeviceID</key>
        <string>WP4N747PKG</string>
        <key>lastSynced</key>
        <date>2025-09-24T21:51:19Z</date>
        <key>modified</key>
        <date>2025-08-01T06:59:46Z</date>
        <key>title</key>
        <string>Jan Nieuwenhuizen - Wikidata</string>
        <key>url</key>
        <string>https://www.wikidata.org/wiki/Q18602659</string>
        <key>isPinned</key>
        <false/>
      </dict>
    </array>
  </dict>
</array>
</plist>
"#;

    const EMPTY_ORION_SYNCED_TABS_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array/>
</plist>
"#;

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_orion_synced_tabs_snapshot() {
        let path = write_temp_fixture("parse-orion-synced-tabs", ORION_SYNCED_TABS_FIXTURE);
        let tabs = parse_orion_synced_snapshot(&path, "current").expect("parse synced tabs");

        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].id, "321B9D93-048B-466C-8067-66129EB2A40E");
        assert_eq!(tabs[0].browser, "orion");
        assert_eq!(tabs[0].source, "current");
        assert_eq!(tabs[0].device_id.as_deref(), Some("WP4N747PKG"));
        assert_eq!(tabs[0].window_name.as_deref(), Some("Window 1"));
        assert_eq!(
            tabs[0].window_id.as_deref(),
            Some("B9E48D83-2884-4362-A4FD-A0747EFA6CBB")
        );
        assert_eq!(tabs[0].title, "Jan Nieuwenhuizen - Wikidata");
        assert_eq!(tabs[0].url, "https://www.wikidata.org/wiki/Q18602659");
        assert!(!tabs[0].pinned);
        assert_eq!(tabs[0].last_synced.as_deref(), Some("2025-09-24T21:51:19Z"));
        assert_eq!(tabs[0].modified.as_deref(), Some("2025-08-01T06:59:46Z"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn picks_newest_non_empty_orion_snapshot() {
        let home = temp_home_dir("latest-non-empty-orion-snapshot");
        let defaults_dir = home.join("Library/Application Support/Orion/Defaults");
        std::fs::create_dir_all(defaults_dir.join("bk_136")).expect("create bk_136");
        std::fs::write(
            defaults_dir.join("bk_136/.local_named_windows.plist"),
            ORION_SYNCED_TABS_FIXTURE,
        )
        .expect("write non-empty fixture");

        std::thread::sleep(Duration::from_millis(20));

        std::fs::create_dir_all(defaults_dir.join("bk_145")).expect("create bk_145");
        std::fs::write(
            defaults_dir.join("bk_145/.local_named_windows.plist"),
            EMPTY_ORION_SYNCED_TABS_FIXTURE,
        )
        .expect("write empty fixture");

        let (snapshot, tabs) = latest_non_empty_orion_snapshot(&defaults_dir)
            .expect("discover snapshots")
            .expect("expected synced snapshot");

        assert!(snapshot.ends_with("bk_136/.local_named_windows.plist"));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].browser, "orion");
        assert_eq!(tabs[0].source, "archived");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_orion_sync_state_does_not_fall_back_to_archived_snapshot() {
        let home = temp_home_dir("current-orion-sync-state");
        let defaults_dir = home.join("Library/Application Support/Orion/Defaults");
        std::fs::create_dir_all(&defaults_dir).expect("create defaults dir");
        std::fs::write(
            defaults_dir.join(".local_named_windows.plist"),
            EMPTY_ORION_SYNCED_TABS_FIXTURE,
        )
        .expect("write current empty fixture");
        std::fs::create_dir_all(defaults_dir.join("bk_136")).expect("create bk_136");
        std::fs::write(
            defaults_dir.join("bk_136/.local_named_windows.plist"),
            ORION_SYNCED_TABS_FIXTURE,
        )
        .expect("write archived fixture");

        let current_tabs =
            list_synced_tabs_from_home(&home, Some("orion"), false).expect("current synced tabs");
        let archived_tabs =
            list_synced_tabs_from_home(&home, Some("orion"), true).expect("archived synced tabs");

        assert!(current_tabs.is_empty());
        assert_eq!(archived_tabs.len(), 1);
        assert_eq!(archived_tabs[0].source, "archived");
    }

    #[test]
    fn rejects_unsupported_synced_browser_filter() {
        let error = list_synced_tabs_from_home(Path::new("/tmp"), Some("brave"), false)
            .expect_err("expected unsupported browser error");
        assert!(error.contains("Synced tabs are not supported for browser 'brave' yet"));
    }

    fn write_temp_fixture(name: &str, contents: &str) -> PathBuf {
        let dir = temp_home_dir(name);
        let path = dir.join("fixture.plist");
        std::fs::write(&path, contents).expect("write fixture");
        path
    }

    fn temp_home_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustab-cli-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
