//! End to end: collector pipeline -> store -> MCP over HTTP -> real MCP client.

use starlist::collector::{DEFAULT_MARKER_REGEX, Pipeline};
use starlist::redact::Redactor;
use starlist::server::LogServer;
use starlist::store::Store;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use rmcp::transport::StreamableHttpClientTransport;
use std::sync::Arc;

fn iso_ago(secs: i64) -> String {
    let t = chrono::DateTime::from_timestamp(starlist::analysis::now() as i64 - secs, 0).unwrap();
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[tokio::test]
async fn agent_finds_bug_and_verifies_fix_over_http() -> anyhow::Result<()> {
    let db = std::env::temp_dir().join(format!("starlist-e2e-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db)?);
    let p = Pipeline::new(store.clone(), Redactor::default(), Some(regex::Regex::new(DEFAULT_MARKER_REGEX)?));

    let mut lines = vec![format!("{} INFO Starting api version 2.0.0", iso_ago(7200))];
    for i in 0..120 {
        lines.push(format!("{} INFO GET /items/{i} 200 user=a{i}@example.com", iso_ago(7000 - i * 50)));
    }
    for i in 0..40 {
        lines.push(format!(
            "{} ERROR charge failed card=4111111111111111\nTraceback (most recent call last):\n  File \"pay.py\", line 3\nValueError: bad amount {i}",
            iso_ago(3000 - i * 30)
        ));
    }
    lines.push(format!("{} INFO Starting api version 2.0.1", iso_ago(1500)));
    p.ingest(&lines);

    let service: StreamableHttpService<LogServer, LocalSessionManager> = StreamableHttpService::new(
        { let s = store.clone(); move || Ok(LogServer::new(s.clone())) },
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, axum::Router::new().nest_service("/mcp", service)).await });

    let client = ClientConfig::default()
        .serve(StreamableHttpClientTransport::from_config(StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"))))
        .await?;
    let call = |name: &'static str, args: serde_json::Value| {
        let client = &client;
        async move {
            let r = client
                .call_tool(CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()))
                .await
                .unwrap();
            let text = r.content[0].as_text().unwrap().text.clone();
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        }
    };

    let news = call("whats_new", serde_json::json!({"window": "2h", "baseline": "24h"})).await;
    let top = &news["findings"][0];
    assert_eq!(top["kind"], "new");
    assert!(top["template"].as_str().unwrap().ends_with("| ValueError"));
    let sample = top["sample"].as_str().unwrap();
    assert!(sample.contains("<CARD>") && !sample.contains("4111"), "card must be redacted: {sample}");

    let verdict = call("verify_fix", serde_json::json!({"pattern_id": top["pattern_id"], "since": "v2.0.1"})).await;
    assert_eq!(verdict["verdict"], "fixed", "{verdict}");

    let _ = client.cancel().await;
    Ok(())
}
