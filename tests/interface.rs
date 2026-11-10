//! The public tool interface: every tool's name, arguments and output fields,
//! checked against docs/tools.md. From 1.0.0 these only grow within a major
//! version (see docs/versioning.md). After an intended change, regenerate the
//! file with `UPDATE_INTERFACE=1 cargo test --test interface` and note the
//! change in CHANGELOG.md.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};
use starlist::collector::{DEFAULT_MARKER_REGEX, Pipeline};
use starlist::redact::Redactor;
use starlist::server::LogServer;
use starlist::store::Store;
use std::collections::BTreeSet;
use std::sync::Arc;

fn iso_ago(secs: i64) -> String {
    let t = chrono::DateTime::from_timestamp(starlist::analysis::now() as i64 - secs, 0).unwrap();
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Every field path in a value: `a`, `a.b`, `items[].c`.
fn paths(v: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                out.insert(p.clone());
                paths(v, &p, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                paths(v, &format!("{prefix}[]"), out);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn tool_interface_matches_docs() -> anyhow::Result<()> {
    let db = std::env::temp_dir().join(format!("starlist-interface-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db)?);
    let p = Pipeline::new(
        store.clone(),
        Redactor::default(),
        Some(regex::Regex::new(DEFAULT_MARKER_REGEX)?),
    );
    let mut lines = vec![format!("{} INFO Starting api version 2.0.0", iso_ago(7200))];
    for i in 0..120 {
        lines.push(format!(
            "{} INFO GET /items/{i} 200 request_id=req-{i} duration_ms={i}",
            iso_ago(7000 - i * 50)
        ));
    }
    lines.push(format!("{} INFO Starting api version 2.0.1", iso_ago(3100)));
    for i in 0..40 {
        lines.push(format!(
            "{} INFO GET /pay/{i} request_id=pay-{i}\n{} ERROR charge failed request_id=pay-{i}\nTraceback (most recent call last):\n  File \"/app/shop/pay.py\", line 3, in charge\nValueError: bad amount {i}",
            iso_ago(3000 - i * 30),
            iso_ago(3000 - i * 30)
        ));
    }
    for i in 0..30 {
        lines.push(format!(
            "{} WARN cache miss for key k{i}",
            iso_ago(1700 - i * 20)
        ));
    }
    lines.push(format!("{} INFO Starting api version 2.0.2", iso_ago(1000)));
    p.ingest(&lines);

    let service: StreamableHttpService<LogServer, LocalSessionManager> = StreamableHttpService::new(
        {
            let s = store.clone();
            move || Ok(LogServer::new(s.clone()))
        },
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().nest_service("/mcp", service)).await
    });
    let client = ClientConfig::default()
        .serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
        ))
        .await?;
    let call = |name: &str, args: Value| {
        let client = &client;
        let name = name.to_string();
        async move {
            let r = client
                .call_tool(
                    CallToolRequestParams::new(name.clone())
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            let text = r.content[0].as_text().unwrap().text.clone();
            serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|e| panic!("{name} did not answer JSON ({e}): {text}"))
        }
    };

    let news = call("whats_new", json!({"window": "1h", "detail": "full"})).await;
    let error_id = news["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["template"].as_str().unwrap_or("").contains("ValueError"))
        .expect("the error is new")["pattern_id"]
        .clone();
    let warn_id = news["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["template"].as_str().unwrap_or("").contains("cache miss"))
        .expect("the warning is new")["pattern_id"]
        .clone();

    // One call per tool, in full detail so every optional field shows.
    let calls: Vec<(&str, Value, Value)> = vec![
        ("diagnose", json!({"detail": "full"}), Value::Null),
        ("whats_new", json!({"window": "1h", "detail": "full"}), news),
        (
            "verify_fix",
            json!({"pattern_id": error_id, "since": "v2.0.2", "detail": "full"}),
            Value::Null,
        ),
        (
            "compare_deploy",
            json!({"at": "v2.0.1", "detail": "full"}),
            Value::Null,
        ),
        ("list_patterns", json!({"detail": "full"}), Value::Null),
        (
            "get_pattern",
            json!({"pattern_id": error_id, "detail": "full"}),
            Value::Null,
        ),
        (
            "get_context",
            json!({"pattern_id": error_id, "before": 2, "after": 2, "detail": "full"}),
            Value::Null,
        ),
        (
            "get_request",
            json!({"request_id": "pay-3", "detail": "full"}),
            Value::Null,
        ),
        (
            "get_release",
            json!({"name": "v2.0.1", "detail": "full"}),
            Value::Null,
        ),
        (
            "add_marker",
            json!({"name": "fix-pay", "note": "retry charges"}),
            Value::Null,
        ),
        ("list_markers", json!({}), Value::Null),
        (
            "set_pattern_state",
            json!({"pattern_id": error_id, "state": "resolved", "note": "fixed"}),
            Value::Null,
        ),
        (
            "merge_patterns",
            json!({"pattern_ids": [error_id, warn_id]}),
            Value::Null,
        ),
        (
            "split_pattern",
            json!({"pattern_id": error_id, "pattern_ids": [warn_id]}),
            Value::Null,
        ),
    ];

    let mut tools = LogServer::new_tools();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    let called: BTreeSet<&str> = calls.iter().map(|c| c.0).collect();
    let listed: BTreeSet<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(called, listed, "every tool needs a call in this test");

    let mut doc = String::from(
        "# Tool interface\n\n\
         Generated by `tests/interface.rs` from a running server; don't edit by hand. \
         Regenerate with `UPDATE_INTERFACE=1 cargo test --test interface`. \
         See [versioning.md](versioning.md) for what may change between releases.\n\n\
         Output fields are listed as paths: `findings[].pattern_id` is a field of each item of `findings`. \
         They are the fields seen with `detail: \"full\"`; brief answers have a subset, and a field \
         with nothing to say may be left out.\n",
    );
    for (name, args, known) in calls {
        let out = if known.is_null() {
            call(name, args).await
        } else {
            known
        };
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let props = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object());
        let required: BTreeSet<&str> = tool
            .input_schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let mut fields = BTreeSet::new();
        paths(&out, "", &mut fields);

        doc.push_str(&format!("\n## `{name}`\n\nArguments:"));
        match props {
            Some(p) if !p.is_empty() => {
                doc.push('\n');
                for k in p.keys() {
                    let req = if required.contains(k.as_str()) {
                        " (required)"
                    } else {
                        ""
                    };
                    doc.push_str(&format!("- `{k}`{req}\n"));
                }
            }
            _ => doc.push_str(" none\n"),
        }
        doc.push_str("\nOutput fields:\n");
        for f in fields {
            doc.push_str(&format!("- `{f}`\n"));
        }
    }

    let _ = client.cancel().await;
    let _ = std::fs::remove_file(&db);

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/tools.md");
    if std::env::var_os("UPDATE_INTERFACE").is_some() {
        std::fs::write(&path, &doc)?;
        return Ok(());
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        want == doc,
        "the tool interface changed. If that's intended, run \
         `UPDATE_INTERFACE=1 cargo test --test interface` and note it in CHANGELOG.md.\n\n{}",
        diff(&want, &doc)
    );
    Ok(())
}

/// The lines only one side has, enough to see what changed.
fn diff(want: &str, got: &str) -> String {
    let w: BTreeSet<&str> = want.lines().collect();
    let g: BTreeSet<&str> = got.lines().collect();
    let mut s = String::new();
    for l in w.difference(&g) {
        s.push_str(&format!("- {l}\n"));
    }
    for l in g.difference(&w) {
        s.push_str(&format!("+ {l}\n"));
    }
    s
}
