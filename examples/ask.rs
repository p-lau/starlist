//! Tiny MCP client that plays the agent: lists tools, or calls one and prints the answer.
//!
//!   cargo run --example ask -- http://127.0.0.1:7341/mcp                    # list tools
//!   cargo run --example ask -- http://127.0.0.1:7341/mcp whats_new '{"window":"1h"}'
//!
//! Set STARLIST_TOKEN when the server requires a bearer token.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| "http://127.0.0.1:7341/mcp".into());
    let mut config = StreamableHttpClientTransportConfig::with_uri(url);
    if let Ok(t) = std::env::var("STARLIST_TOKEN") {
        config = config.auth_header(t);
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ClientConfig::default().serve(transport).await?;
    match args.next() {
        None => {
            for t in client.list_all_tools().await? {
                println!("{:<16} {}", t.name, t.description.as_deref().unwrap_or(""));
            }
        }
        Some(tool) => {
            let json = args.next().unwrap_or_else(|| "{}".into());
            let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&json)?;
            let res = client.call_tool(CallToolRequestParams::new(tool).with_arguments(map)).await?;
            if res.is_error == Some(true) {
                print!("ERROR: ");
            }
            for c in &res.content {
                if let Some(t) = c.as_text() {
                    println!("{}", t.text);
                }
            }
        }
    }
    let _ = client.cancel().await;
    Ok(())
}
