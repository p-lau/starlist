//! Bearer-token auth on the HTTP endpoint.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use starlist::store::Store;
use std::sync::Arc;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

async fn serve(token: Option<&str>) -> String {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let db = std::env::temp_dir().join(format!("starlist-auth-{}-{n}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db).unwrap());
    let router = starlist::http::router(store, token.map(str::to_string), vec![]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{addr}/mcp")
}

async fn post_status(url: &str, auth: Option<&str>) -> u16 {
    let mut req = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
    if let Some(a) = auth {
        req = req.header("authorization", a);
    }
    req.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn rejects_missing_or_wrong_token() {
    let url = serve(Some(TOKEN)).await;
    assert_eq!(post_status(&url, None).await, 401);
    assert_eq!(post_status(&url, Some("Bearer nope")).await, 401);
    assert_eq!(
        post_status(&url, Some(&format!("Basic {TOKEN}"))).await,
        401
    );
}

#[tokio::test]
async fn mcp_client_with_token_works() {
    let url = serve(Some(TOKEN)).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(TOKEN),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();
    let r = client
        .call_tool(
            CallToolRequestParams::new("list_markers").with_arguments(serde_json::Map::new()),
        )
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true));
    let _ = client.cancel().await;
}

#[tokio::test]
async fn no_token_needed_on_localhost_by_default() {
    let url = serve(None).await;
    assert_ne!(post_status(&url, None).await, 401);
}

#[test]
fn generated_tokens_are_long_and_random() {
    let (a, b) = (
        starlist::http::generate_token().unwrap(),
        starlist::http::generate_token().unwrap(),
    );
    assert_eq!(a.len(), 64);
    assert_ne!(a, b);
}
