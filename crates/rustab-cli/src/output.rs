pub fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let rendered =
        serde_json::to_string_pretty(value).map_err(|e| format!("failed to render JSON: {e}"))?;
    println!("{rendered}");
    Ok(())
}
