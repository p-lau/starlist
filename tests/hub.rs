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

#[tokio::test]
async fn push_route_checks_the_bearer_token() {
    let dir = tmp("hub");
    let hub = Arc::new(Hub::open(&dir, Some(20 << 20)).unwrap());
    let api_token = Hub::add_token(&dir, "api").unwrap();
    let base = serve(hub, HttpConfig::default()).await;
    let body = |service: &str| {
        json!({"protocol": 1, "service": service, "instance": "i1", "entries": []}).to_string()
    };
    let post = |auth: Option<String>, body: String| {
        let url = format!("{base}/hub/v1/push");
        async move {
            let mut r = reqwest::Client::new()
                .post(url)
                .header("Content-Type", "application/json");
            if let Some(a) = auth {
                r = r.header("Authorization", a);
            }
            r.body(body).send().await.unwrap().status().as_u16()
        }
    };
    assert_eq!(post(None, body("api")).await, 401);
    assert_eq!(post(Some("Bearer nope".into()), body("api")).await, 401);
    assert_eq!(post(Some(api_token.clone()), body("api")).await, 401);
    assert_eq!(
        post(Some(format!("Bearer {api_token}")), body("other")).await,
        403
    );
    assert_eq!(
        post(Some(format!("Bearer {api_token}")), "not json".into()).await,
        400
    );
    assert_eq!(
        post(Some(format!("Bearer {api_token}")), body("api")).await,
        200
    );
}

#[tokio::test]
async fn the_hub_pruner_folds_every_service() {
    let dir = tmp("hub");
    let hub = Arc::new(Hub::open(&dir, Some(20 << 20)).unwrap());
    const D: f64 = 86400.0;
    let mut ids = vec![];
    for svc in ["api", "pricing"] {
        let s = hub.store_or_create(svc).unwrap();
        let old: Vec<Event> = (0..5)
            .map(|_| ev(&format!("ERROR {svc} old failure"), 20.0 * D))
            .collect();
        let recent: Vec<Event> = (0..3)
            .map(|_| ev(&format!("ERROR {svc} recent failure"), 60.0))
            .collect();
        s.record(&old).unwrap();
        s.record(&recent).unwrap();
        ids.push((s, old[0].pattern_id.clone(), recent[0].pattern_id.clone()));
    }
    let now = starlist::analysis::now();
    for (s, old, _) in &ids {
        assert_eq!(s.count(old, now - 30.0 * D, now).unwrap(), 5);
    }
    // Retention 7 days, history 14 days: the 20-day-old counts go, the rest stay.
    hub.spawn_pruner(7.0 * D, 14.0 * D, 0);
    for (s, old, recent) in &ids {
        let mut gone = false;
        for _ in 0..100 {
            if s.count(old, now - 30.0 * D, now).unwrap() == 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(gone, "old counts were not pruned");
        assert_eq!(s.count(recent, now - 30.0 * D, now + 60.0).unwrap(), 3);
    }
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

// ---- hub alerts and metrics ----

use starlist::alerts::AlertConfig;
use starlist::hub::spawn_alerts;
use std::time::Duration;

fn fast(notify_url: Option<String>) -> AlertConfig {
    AlertConfig {
        interval: Duration::from_millis(100),
        debounce_secs: 0.0,
        min_gap_secs: 0.0,
        cooldown_secs: 1800.0,
        window: "5m".into(),
        notify_url,
    }
}

/// A hub with a quiet day of traffic for each named service.
fn quiet_hub(services: &[&str]) -> Arc<Hub> {
    let hub = Arc::new(Hub::open(tmp("hub"), Some(20 << 20)).unwrap());
    for svc in services {
        let evs: Vec<Event> = (0..300)
            .map(|i| ev("INFO GET /items 200", 86000.0 - i as f64 * 280.0))
            .collect();
        hub.store_or_create(svc).unwrap().record(&evs).unwrap();
    }
    hub
}

fn crash(hub: &Hub, svc: &str, text: &str) {
    hub.store_or_create(svc)
        .unwrap()
        .record(&[ev(text, 0.0)])
        .unwrap();
}

fn api_cfg() -> HttpConfig {
    HttpConfig {
        api: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_new_error_in_one_service_reaches_the_hub_alert_stream() {
    let hub = quiet_hub(&["api", "pricing"]);
    let alerts = spawn_alerts(hub.clone(), fast(None));
    assert!(hub.alerts().is_some());
    let base = serve(hub.clone(), api_cfg()).await;

    let mut stream = reqwest::get(format!("{base}/api/v1/alerts/stream"))
        .await
        .unwrap();
    assert_eq!(stream.status().as_u16(), 200);
    let _keep = alerts;
    crash(&hub, "api", "ERROR upstream exploded\nPanicError: boom");

    let mut text = String::new();
    let got = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            text.push_str(&String::from_utf8_lossy(&chunk));
            if text.contains("\n\n") && text.contains("data:") {
                break;
            }
        }
    })
    .await;
    assert!(got.is_ok(), "no alert within 10s: {text:?}");
    assert!(text.contains("event: alert"), "{text}");
    let data = text.lines().find_map(|l| l.strip_prefix("data:")).unwrap();
    let v: Value = serde_json::from_str(data.trim()).unwrap();
    assert_eq!(v["service"], "api", "{v}");
    assert_eq!(v["source"], "starlist");
    assert!(v["summary"].as_str().unwrap().starts_with("api: "), "{v}");
    assert!(!v["findings"].as_array().unwrap().is_empty());

    let (c, v) = get(format!("{base}/api/v1/alerts")).await;
    assert_eq!(c, 200);
    assert_eq!(v["enabled"], true);
    assert_eq!(v["alert"]["service"], "api");
}

/// Accepts one POST and sends its JSON body down the channel.
fn webhook_server() -> (String, std::sync::mpsc::Receiver<Value>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { return };
            let mut buf = vec![];
            let mut tmp = [0u8; 4096];
            let body = loop {
                let n = c.read(&mut tmp).unwrap_or(0);
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        break body[..len].to_string();
                    }
                }
                if n == 0 {
                    break String::new();
                }
            };
            let _ =
                c.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            if let Ok(v) = serde_json::from_str(&body) {
                let _ = tx.send(v);
            }
        }
    });
    (url, rx)
}

#[tokio::test]
async fn a_new_error_is_posted_to_the_webhook_with_its_service() {
    let (url, rx) = webhook_server();
    let hub = quiet_hub(&["api"]);
    let _alerts = spawn_alerts(hub.clone(), fast(Some(url)));
    crash(
        &hub,
        "api",
        "FATAL worker crashed\nPanicError: out of memory",
    );

    let v = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10)))
        .await
        .unwrap()
        .expect("no webhook call within 10s");
    assert_eq!(v["service"], "api", "{v}");
    assert_eq!(v["source"], "starlist");
    assert!(v["summary"].as_str().unwrap().starts_with("api: "), "{v}");
    assert!(v["findings"].is_array());
}

#[tokio::test]
async fn the_same_pattern_does_not_alert_again_within_the_cooldown() {
    let hub = quiet_hub(&["api"]);
    let alerts = spawn_alerts(hub.clone(), fast(None));
    let mut rx = alerts.subscribe();
    crash(&hub, "api", "ERROR payment failed\nKeyError: 'currency'");
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("no first alert")
        .unwrap();
    assert_eq!(first.service.as_deref(), Some("api"));

    // More of the same pattern, over several watcher ticks: quiet.
    for _ in 0..3 {
        crash(&hub, "api", "ERROR payment failed\nKeyError: 'currency'");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(rx.try_recv().is_err(), "second alert inside the cooldown");
}

#[tokio::test]
async fn alerts_are_404_and_disabled_without_a_watcher() {
    let hub = quiet_hub(&["api"]);
    assert!(hub.alerts().is_none());
    let base = serve(hub, api_cfg()).await;
    let (c, _) = get(format!("{base}/api/v1/alerts/stream")).await;
    assert_eq!(c, 404);
    let (c, v) = get(format!("{base}/api/v1/alerts")).await;
    assert_eq!(c, 200);
    assert_eq!(v["enabled"], false);
    assert!(v["alert"].is_null());
}

#[cfg(feature = "prometheus")]
#[tokio::test]
async fn hub_metrics_label_every_series_with_its_service() {
    let hub = quiet_hub(&["api", "pricing"]);
    let base = serve(
        hub.clone(),
        HttpConfig {
            api: true,
            prometheus: true,
            ..Default::default()
        },
    )
    .await;
    let r = reqwest::get(format!("{base}/metrics")).await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let text = r.text().await.unwrap();
    for svc in ["api", "pricing"] {
        assert!(
            text.contains(&format!("starlist_events_total{{service=\"{svc}\"")),
            "{text}"
        );
        assert!(
            text.contains(&format!("starlist_db_bytes{{service=\"{svc}\"}} ")),
            "{text}"
        );
    }
    for l in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        assert!(l.contains("service=\""), "series without service: {l}");
    }

    let off = serve(quiet_hub(&["api"]), api_cfg()).await;
    let (c, _) = get(format!("{off}/metrics")).await;
    assert_eq!(c, 404);
}

// ---- hub alerts: late services, auth, status, flags ----

async fn next_services(
    rx: &mut tokio::sync::broadcast::Receiver<starlist::alerts::Alert>,
    want: &[&str],
) -> Vec<String> {
    let mut seen: Vec<String> = vec![];
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while !want.iter().all(|w| seen.iter().any(|s| s == w)) {
            if let Ok(a) = rx.recv().await {
                seen.extend(a.service);
            }
        }
    })
    .await;
    seen
}

#[tokio::test]
async fn a_service_that_first_pushes_after_start_still_gets_alerts() {
    let hub = quiet_hub(&["api"]);
    let alerts = spawn_alerts(hub.clone(), fast(None));
    let mut rx = alerts.subscribe();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let late = hub.store_or_create("late").unwrap();
    let evs: Vec<Event> = (0..300)
        .map(|i| ev("INFO GET /items 200", 86000.0 - i as f64 * 280.0))
        .collect();
    late.record(&evs).unwrap();
    crash(
        &hub,
        "late",
        "ERROR late service exploded\nPanicError: boom",
    );
    let seen = next_services(&mut rx, &["late"]).await;
    assert!(seen.iter().any(|s| s == "late"), "{seen:?}");
}

#[tokio::test]
async fn the_same_error_in_two_services_alerts_once_per_service() {
    let hub = quiet_hub(&["api", "pricing"]);
    let alerts = spawn_alerts(hub.clone(), fast(None));
    let mut rx = alerts.subscribe();
    for svc in ["api", "pricing"] {
        crash(&hub, svc, "ERROR cache down\nPanicError: redis gone");
    }
    let seen = next_services(&mut rx, &["api", "pricing"]).await;
    for svc in ["api", "pricing"] {
        assert!(seen.iter().any(|s| s == svc), "{svc} missing in {seen:?}");
    }
}

#[tokio::test]
async fn hub_alert_routes_need_the_token() {
    let hub = quiet_hub(&["api"]);
    let _alerts = spawn_alerts(hub.clone(), fast(None));
    let base = serve(
        hub,
        HttpConfig {
            api: true,
            #[cfg(feature = "prometheus")]
            prometheus: true,
            token: Some("s3cret".into()),
            ..Default::default()
        },
    )
    .await;
    let mut paths = vec!["/api/v1/alerts", "/api/v1/alerts/stream"];
    if cfg!(feature = "prometheus") {
        paths.push("/metrics");
    }
    let client = reqwest::Client::new();
    for p in paths {
        let r = client.get(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 401, "{p}");
        let r = client
            .get(format!("{base}{p}"))
            .bearer_auth("s3cret")
            .send()
            .await
            .unwrap();
        assert_ne!(r.status().as_u16(), 401, "{p}");
    }
}

#[tokio::test]
async fn hub_status_reports_whether_alerts_are_on() {
    let hub = quiet_hub(&["api"]);
    let base = serve(hub.clone(), api_cfg()).await;
    let (c, v) = get(format!("{base}/api/v1/status?service=api")).await;
    assert_eq!(c, 200);
    assert_eq!(v["alerts"], false, "{v}");
    // The per-service router above is cached; enabling alerts must rebuild it.
    let _alerts = spawn_alerts(hub, fast(None));
    let (c, v) = get(format!("{base}/api/v1/status?service=api")).await;
    assert_eq!(c, 200);
    assert_eq!(v["alerts"], true, "{v}");
}

#[cfg(feature = "prometheus")]
#[test]
fn an_unreadable_service_is_skipped_in_hub_metrics() {
    let mk = |name: &str| {
        let p = tmp(name);
        let s = Arc::new(Store::open(&p).unwrap());
        s.record(&[ev("INFO ok", 1.0)]).unwrap();
        (p, s)
    };
    let (_, good) = mk("good.db");
    let (bad_path, bad) = mk("bad.db");
    let conn = rusqlite::Connection::open(&bad_path).unwrap();
    conn.execute_batch("DROP TABLE patterns").unwrap();
    drop(conn);
    let stores = vec![("bad".to_string(), bad), ("good".to_string(), good)];
    let text = starlist::prometheus::render_services(&stores, starlist::analysis::now()).unwrap();
    assert!(text.contains("service=\"good\""), "{text}");
    assert!(!text.contains("service=\"bad\""), "{text}");
}

#[tokio::test]
async fn no_alerts_flag_turns_the_stream_off() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tmp("flaghub");
    let _child = Kill(
        std::process::Command::new(env!("CARGO_BIN_EXE_starlist"))
            .args(["hub", "--no-alerts", "--http"])
            .arg(format!("127.0.0.1:{port}"))
            .arg("--dir")
            .arg(&dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let base = format!("http://127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = reqwest::get(format!("{base}/api/v1/alerts/stream")).await {
            assert_eq!(r.status().as_u16(), 404);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "hub never came up");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, v) = get(format!("{base}/api/v1/alerts")).await;
    assert_eq!(v["enabled"], false, "{v}");
}
