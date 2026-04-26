use rustab_protocol::{parse_tab_id, parse_window_id, TabRef, WindowRef};
use std::io::{BufRead, IsTerminal};

/// Collect tab IDs from args, or read from stdin (one per line, first
/// tab-delimited field, so `rustab list | rustab close` works).
pub fn collect_tab_ids(mut args: Vec<String>) -> Vec<String> {
    if !args.is_empty() {
        return args;
    }

    if std::io::stdin().is_terminal() {
        return args;
    }

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(id) = line.split('\t').next() {
            let id = id.trim();
            if !id.is_empty() {
                args.push(id.to_string());
            }
        }
    }

    args
}

#[derive(Debug, Clone, Copy)]
pub enum WindowArg<'a> {
    Raw(u64),
    Scoped(WindowRef<'a>),
}

pub fn parse_window_arg(value: &str) -> Result<WindowArg<'_>, String> {
    if let Ok(window_id) = value.parse::<u64>() {
        return Ok(WindowArg::Raw(window_id));
    }

    parse_window_id(value).map(WindowArg::Scoped).ok_or_else(|| {
        format!(
            "Invalid window ID format: {value} (expected prefix.pid.w.id, prefix.w.id, or raw browser window id)"
        )
    })
}

pub fn parse_tab_ids(tab_ids: &[String]) -> Result<Vec<TabRef<'_>>, String> {
    tab_ids
        .iter()
        .map(|id_str| {
            parse_tab_id(id_str).ok_or_else(|| {
                format!("Invalid tab ID format: {id_str} (expected prefix.pid.id, e.g. c.4242.123)")
            })
        })
        .collect()
}

pub fn validate_move_index(index: i64) -> Result<(), String> {
    if index < -1 {
        return Err("move index must be -1 or greater".to_string());
    }
    Ok(())
}

pub fn validate_open_index(index: Option<i64>) -> Result<(), String> {
    if matches!(index, Some(index) if index < 0) {
        return Err("open index must be 0 or greater".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
