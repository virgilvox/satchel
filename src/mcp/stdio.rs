use anyhow::Result;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::rag::Database;
use crate::embed::Embedder;
use super::{JsonRpcRequest, JsonRpcResponse};

pub async fn serve(db: Database, embedder: Embedder) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    eprintln!("[satchel] MCP server running on stdio");

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[satchel] Parse error: {e}");
                continue;
            }
        };

        let result = super::handle_request(&request, &db, &embedder).await;

        if request.id.is_none() {
            continue;
        }

        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(result),
            error: None,
        };

        let response_str = serde_json::to_string(&response)?;
        stdout.write_all(response_str.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}
