use tardy::ingest::Ingestor;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::env::args()
        .nth(1)
        .ok_or("usage: ingest-preview SOURCE_ID")?;
    let plans = Ingestor::bundled()?.preview(&source).await?;
    println!("{}", serde_json::to_string_pretty(&plans)?);
    Ok(())
}
