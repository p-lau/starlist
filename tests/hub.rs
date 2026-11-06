//! The hub: collectors push, replicas add up, requests cross services (issue #25).

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use starlist::http::{HttpConfig, hub_app};
use starlist::hub::Hub;
use starlist::pattern::{Event, parse_event};
use starlist::push::{HubTarget, instance_id, push_once};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::path::PathBuf;
use std::sync::Arc;

fn tmp(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "starlist-hub-{}-{}-{name}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

fn collector(no_raw: bool) -> Arc<Store> {
    let p = tmp("c.db");
    let _ = std::fs::remove_file(&p);
    let s = Arc::new(Store::open(p).unwrap());
    if no_raw {
        s.set_raw_cap(None).unwrap();
    }
    s.enable_outbox(8 << 20, no_raw);
    s
}

fn ev(text: &str, ago: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(starlist::analysis::now() - ago);
    e
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

async fn push(store: &Arc<Store>, target: &HubTarget) -> Result<usize, String> {
    let (s, t) = (store.clone(), target.clone());
    tokio::task::spawn_blocking(move || {
        let inst = instance_id(&s)?;
        let a = agent();
        let mut total = 0;
        loop {
            let n = push_once(&s, &t, &inst, &a)?;
            if n == 0 {
                return Ok(total);
            }
            total += n;
        }
    })
    .await
    .unwrap()
}

async fn serve(hub: Arc<Hub>, cfg: HttpConfig) -> String {
    let router = hub_app(hub, cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{addr}")
}

async fn get(url: String) -> (u16, Value) {
    let r = reqwest::get(url).await.unwrap();
    let code = r.status().as_u16();
    let text = r.text().await.unwrap();
    (
        code,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn collectors_push_and_the_hub_answers_across_services() {
    let dir = tmp("hub");
    let hub = Arc::new(Hub::open(&dir, Some(20 << 20)).unwrap());
    let base = serve(
        hub.clone(),
        HttpConfig {
            api: true,
            ..Default::default()
        },
    )
    .await;
    let api_token = Hub::add_token(&dir, "api").unwrap();
    let pricing_token = Hub::add_token(&dir, "pricing").unwrap();
    let target = |svc: &str, token: &str| HubTarget {
        url: base.clone(),
        service: svc.into(),
        token: token.into(),
    };

    // Two replicas of "api" and one "pricing", one request through both.
    let api1 = collector(false);
    let api2 = collector(false);
    let pricing = collector(false);
    for (s, n) in [(&api1, 30), (&api2, 12)] {
        let evs: Vec<Event> = (0..n)
            .map(|i| ev("ERROR upstream timeout calling pricing", 600.0 - i as f64))
            .collect();
        s.record(&evs).unwrap();
        s.add_deploy("v7", starlist::analysis::now() - 3600.0, None)
            .unwrap();
    }
    api1.record(&[
        ev(
            "INFO GET /checkout trace_id=4bf92f3577b34da6a3ce929d0e0e4736 request_id=api-1",
            30.0,
        ),
        ev(
            "WARN checkout failed trace_id=4bf92f3577b34da6a3ce929d0e0e4736 request_id=api-1",
            29.5,
        ),
    ])
    .unwrap();
    pricing
        .record(&[
            ev(
                "INFO pricing cart trace_id=4bf92f3577b34da6a3ce929d0e0e4736",
                29.9,
            ),
            ev(
                "ERROR KeyError: 'currency' trace_id=4bf92f3577b34da6a3ce929d0e0e4736",
                29.8,
            ),
        ])
        .unwrap();
    pricing.set_meta("env", Some("production")).unwrap();

    // Wrong and mismatched tokens are refused; nothing is lost.
    assert!(
        push(&api1, &target("api", "nope"))
            .await
            .unwrap_err()
            .contains("401")
    );
    assert!(
        push(&api1, &target("api", &pricing_token))
            .await
            .unwrap_err()
            .contains("403")
    );
    assert!(api1.outbox_len().unwrap().0 > 0);

    assert!(push(&api1, &target("api", &api_token)).await.unwrap() > 0);
    assert!(push(&api2, &target("api", &api_token)).await.unwrap() > 0);
    assert!(
        push(&pricing, &target("pricing", &pricing_token))
            .await
            .unwrap()
            > 0
    );
    assert_eq!(api1.outbox_len().unwrap().0, 0);
    assert_eq!(hub.names(), vec!["api", "pricing"]);

    // Replicas add up under the same pattern ID, and one deploy is one deploy.
    let id = ev("ERROR upstream timeout calling pricing", 0.0).pattern_id;
    let api = hub.store("api").unwrap();
    assert_eq!(api.pattern(&id).unwrap().unwrap().total, 42);
    assert_eq!(api.deploys().unwrap().len(), 1);
    assert_eq!(
        hub.store("pricing")
            .unwrap()
            .meta("env")
            .unwrap()
            .as_deref(),
        Some("production")
    );

    // The API, per service and across services.
    let (c, v) = get(format!("{base}/api/v1/services")).await;
    assert_eq!(c, 200);
    assert_eq!(v["services"].as_array().unwrap().len(), 2);
    let (c, v) = get(format!("{base}/api/v1/patterns")).await;
    assert_eq!(c, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("api, pricing"));
    let (c, v) = get(format!("{base}/api/v1/patterns?service=api")).await;
    assert_eq!(c, 200);
    assert!(
        v["patterns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == id)
    );
    let (c, v) = get(format!("{base}/api/v1/patterns/{id}")).await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(v["total"], 42);
    let (c, v) = get(format!("{base}/api/v1/status?service=api")).await;
    assert_eq!(c, 200);
    assert_eq!(v["service"], "api");
    let (c, v) = get(format!("{base}/api/v1/status")).await;
    assert_eq!(c, 200);
    assert_eq!(v["hub"], true);
    let (c, _) = get(format!("{base}/api/v1/status?service=nope")).await;
    assert_eq!(c, 404);

    let (c, v) = get(format!(
        "{base}/api/v1/requests/4bf92f3577b34da6a3ce929d0e0e4736"
    ))
    .await;
    assert_eq!(c, 200, "{v}");
    let svcs: Vec<&str> = v["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["service"].as_str().unwrap())
        .collect();
    assert_eq!(svcs, vec!["api", "pricing", "pricing", "api"]);
    assert_eq!(v["services"].as_array().unwrap().len(), 2);

    let (c, v) = get(format!("{base}/api/v1/whats-new?window=1h&baseline=24h")).await;
    assert_eq!(c, 200, "{v}");
    let f = v["findings"].as_array().unwrap();
    assert!(f.iter().any(|x| x["service"] == "api"));
    assert!(f.iter().any(|x| x["service"] == "pricing"));
    assert_eq!(f[0]["pattern"]["level"], "ERROR");

    // MCP: the collector tools with `service`, plus list_services.
    let client = ClientConfig::default()
        .serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp")),
        ))
        .await
        .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools[0].name, "list_services");
    let wn = tools.iter().find(|t| t.name == "whats_new").unwrap();
    assert!(wn.input_schema["properties"]["service"].is_object());
    let call = |name: &'static str, args: Value| {
        let client = &client;
        async move {
            let r = client
                .call_tool(
                    CallToolRequestParams::new(name)
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            (
                r.is_error.unwrap_or(false),
                r.content[0].as_text().unwrap().text.clone(),
            )
        }
    };
    let (e, t) = call("list_services", json!({})).await;
    assert!(!e && t.contains("pricing"), "{t}");
    let (e, t) = call(
        "get_request",
        json!({"request_id": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"}),
    )
    .await;
    assert!(!e, "{t}");
    assert!(t.contains("[pricing]") && t.contains("[api]"), "{t}");
    assert!(t.contains("clock"), "{t}");
    let (e, t) = call("whats_new", json!({})).await;
    assert!(!e, "{t}");
    let v: Value = serde_json::from_str(&t).unwrap();
    assert!(v["by_service"]["api"].is_object() && v["by_service"]["pricing"].is_object());
    let (e, t) = call("get_pattern", json!({"pattern_id": id})).await;
    assert!(!e && t.contains("upstream timeout"), "{t}");
    let (e, t) = call("list_markers", json!({})).await;
    assert!(e && t.contains("Pass service"), "{t}");
    let (e, t) = call("list_markers", json!({"service": "api"})).await;
    assert!(!e && t.contains("v7"), "{t}");
}

#[tokio::test]
async fn pushes_are_idempotent_and_survive_an_outage() {
    let dir = tmp("hub");
    let hub = Arc::new(Hub::open(&dir, Some(20 << 20)).unwrap());
    let token = Hub::add_token(&dir, "worker").unwrap();
    let c = collector(true);
    c.record(&[ev("ERROR job failed request_id=job-1234", 60.0)])
        .unwrap();

    // The hub isn't running yet: the push fails and the outbox keeps it.
    let down = HubTarget {
        url: "http://127.0.0.1:9".into(),
        service: "worker".into(),
        token: token.clone(),
    };
    assert!(push(&c, &down).await.is_err());
    c.record(&[ev("ERROR job failed request_id=job-5678", 30.0)])
        .unwrap();
    starlist::issues::merge(
        &c,
        &[
            ev("ERROR job failed request_id=job-1", 0.0).pattern_id,
            ev("WARN job slow", 0.0).pattern_id,
        ],
    )
    .ok();
    assert_eq!(c.outbox_len().unwrap().0, 2);

    let base = serve(
        hub.clone(),
        HttpConfig {
            api: true,
            ..Default::default()
        },
    )
    .await;
    let up = HubTarget {
        url: base.clone(),
        ..down
    };
    // The same batch applied twice counts once.
    let (s, t) = (c.clone(), up.clone());
    let body = tokio::task::spawn_blocking(move || {
        let rows = s.outbox_batch(1 << 20).unwrap();
        let entries: Vec<Value> = rows
            .iter()
            .map(|(seq, b)| json!({"seq": seq, "item": serde_json::from_str::<Value>(b).unwrap()}))
            .collect();
        let inst = instance_id(&s).unwrap();
        let body =
            json!({"protocol": 1, "service": "worker", "instance": inst, "entries": entries});
        for _ in 0..2 {
            let mut r = agent()
                .post(format!("{}/hub/v1/push", t.url))
                .header("Authorization", &format!("Bearer {}", t.token))
                .header("Content-Type", "application/json")
                .send(body.to_string())
                .unwrap();
            assert_eq!(r.status().as_u16(), 200);
            let _ = r.body_mut().read_to_string();
        }
        body
    })
    .await
    .unwrap();
    assert!(body["entries"].as_array().unwrap().len() == 2);
    let w = hub.store("worker").unwrap();
    let id = ev("ERROR job failed request_id=job-1", 0.0).pattern_id;
    assert_eq!(w.pattern(&id).unwrap().unwrap().total, 2);

    // A collector with --no-raw sends no lines for the hub's raw buffer.
    assert_eq!(w.raw_span().unwrap().0, 0);
    let (code, _) = get(format!("{base}/api/v1/requests/job-1234")).await;
    assert_eq!(code, 404);

    // The real pusher sends the same entries; the hub has them already, so
    // it only acknowledges them.
    assert_eq!(push(&c, &up).await.unwrap(), 2);
    assert_eq!(c.outbox_len().unwrap().0, 0);
    assert_eq!(w.pattern(&id).unwrap().unwrap().total, 2);
}

#[test]
fn outbox_drops_the_oldest_beyond_its_cap() {
    let p = tmp("c.db");
    let s = Store::open(&p).unwrap();
    s.enable_outbox(1 << 20, false);
    let big = "x".repeat(3000);
    for i in 0..600 {
        s.record(&[ev(&format!("INFO bulk {big} n={i}"), 10.0)])
            .unwrap();
    }
    let (_, bytes) = s.outbox_len().unwrap();
    assert!(bytes <= 1 << 20, "{bytes}");
    let dropped: i64 = s.meta("hub_dropped").unwrap().unwrap().parse().unwrap();
    assert!(dropped > 0);
}

#[test]
fn trace_ids_win_over_request_ids() {
    assert_eq!(
        starlist::request::find("GET /x request_id=api-1 trace_id=abcd1234ef").as_deref(),
        Some("abcd1234ef")
    );
    assert_eq!(
        starlist::request::find(r#"{"request_id":"api-1","trace_id":"abcd1234ef","msg":"x"}"#)
            .as_deref(),
        Some("abcd1234ef")
    );
    assert!(!starlist::push::valid_service("../etc"));
    assert!(starlist::push::valid_service("checkout-api.eu"));
}
