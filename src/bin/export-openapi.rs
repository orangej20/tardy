fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(&tardy::openapi::document())?;
    match std::env::args_os().nth(1) {
        Some(path) => std::fs::write(path, format!("{json}\n"))?,
        None => println!("{json}"),
    }
    Ok(())
}
