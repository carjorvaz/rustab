use rustab_protocol::{
    browser_prefix, is_pid_alive, parse_socket_name, read_message, socket_dir, validate_socket_dir,
    write_message, RpcRequest, RpcResponse, TabRef, WindowRef, REQUEST_TIMEOUT_SECS,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::PathBuf;
use tokio::net::UnixStream;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserSocket {
    pub browser: String,
    pub pid: u32,
    pub path: PathBuf,
}

pub fn discover_sockets(browser_filter: Option<&str>) -> Vec<BrowserSocket> {
    let dir = socket_dir();
    if !dir.exists() {
        return vec![];
    }
    if let Err(error) = validate_socket_dir(&dir) {
        eprintln!("Ignoring socket directory {}: {error}", dir.display());
        return vec![];
    }

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

pub async fn send_rpc<T>(socket: &BrowserSocket, request: &RpcRequest) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let request = serde_json::to_value(request).map_err(|e| format!("encode request: {e}"))?;
    let response = send_request(&socket.path, &request).await?;
    let response: RpcResponse<T> =
        serde_json::from_value(response).map_err(|e| format!("invalid response: {e}"))?;

    response.into_result()
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
        std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
        read_message(&mut reader),
    )
    .await
    .map_err(|_| "timeout".to_string())?
    .map_err(|e| format!("read: {e}"))
}

pub fn resolve_socket<'a>(
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

pub fn resolve_socket_for_tab_ref<'a>(
    sockets: &'a [BrowserSocket],
    tab_ref: TabRef<'_>,
) -> Result<&'a BrowserSocket, String> {
    resolve_socket(sockets, tab_ref.prefix, tab_ref.mediator_pid)
}

pub fn resolve_socket_for_window_ref<'a>(
    sockets: &'a [BrowserSocket],
    window_ref: WindowRef<'_>,
) -> Result<&'a BrowserSocket, String> {
    resolve_socket(sockets, window_ref.prefix, window_ref.mediator_pid)
}

pub fn same_socket(left: &BrowserSocket, right: &BrowserSocket) -> bool {
    left.browser == right.browser && left.pid == right.pid && left.path == right.path
}

pub fn socket_for_raw_window_id<'a>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustab_protocol::{TabRef, WindowRef};

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
}
