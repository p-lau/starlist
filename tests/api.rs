//! The JSON API under /api/v1 (issue #21).

use serde_json::{Value, json};
use starlist::http::{HttpConfig, app};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn ev(text: &str, ago: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(starlist::analysis::now() - ago);
    e
}

/// A day of steady INFO traffic, a deploy 30 minutes ago, and an error that
/// started with it, logged inside one request.
fn seeded_store() -> Arc<Store> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let db = std::env::temp_dir().join(format!(
        "starlist-api-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&db);
    let s = Arc::new(Store::open(&db).unwrap());
    let mut evs = vec![];
    for i in 0..200 {
        evs.push(ev(
            &format!("INFO GET /items/{i} 200 request_id=req-{i:04}"),
            86000.0 - i as f64 * 400.0,
        ));
    }
    for i in 0..12 {
        evs.push(ev(
            "INFO pricing cart in EUR request_id=badreq-0001",
            1500.0 - i as f64 * 60.0,
        ));
        evs.push(ev(
            &format!(
                "ERROR checkout failed request_id=badreq-0001\nTraceback (most recent call last):\n  File \"shop/pricing.py\", line 41, in price_cart\nKeyError: 'currency{i}'"
            ),
            1499.0 - i as f64 * 60.0,
        ));
    }
    s.record(&evs).unwrap();
    s.add_deploy("v2.0.0", starlist::analysis::now() - 1800.0, None)
        .unwrap();
    s
}

async fn serve(cfg: HttpConfig) -> String {
    let router = app(seeded_store(), cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{addr}")
}

fn open_cfg() -> HttpConfig {
    HttpConfig {
        api: true,
        ..Default::default()
    }
}

async fn get(url: &str) -> (u16, Value) {
    let r = reqwest::get(url).await.unwrap();
    let code = r.status().as_u16();
    let body = r.text().await.unwrap();
    (
        code,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

#[tokio::test]
async fn reads_status_patterns_and_timeline() {
    let base = serve(open_cfg()).await;

    let (code, st) = get(&format!("{base}/api/v1/status")).await;
    assert_eq!(code, 200, "{st}");
    assert_eq!(st["current_release"]["name"], "v2.0.0");
    assert_eq!(st["patterns"], 3);
    assert_eq!(st["last_hour"]["errors"], 12);
    assert!(st["raw"]["events"].as_i64().unwrap() > 0);

    let (code, ps) = get(&format!("{base}/api/v1/patterns?min_level=ERROR")).await;
    assert_eq!(code, 200, "{ps}");
    assert_eq!(ps["total"], 1);
    let p = &ps["patterns"][0];
    assert_eq!(p["level"], "ERROR");
    assert_eq!(p["count"], 12);
    assert_eq!(p["exception"], "KeyError");
    assert_eq!(p["first_release"], "v2.0.0");
    assert_eq!(p["trend"].as_array().unwrap().len(), 24);
    assert!(p["first_seen"].as_str().unwrap().ends_with('Z'));
    let id = p["id"].as_str().unwrap().to_string();

    let (code, one) = get(&format!("{base}/api/v1/patterns/{id}")).await;
    assert_eq!(code, 200, "{one}");
    assert_eq!(one["count_last_hour"], 12);
    assert!(
        one["samples"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Traceback")
    );
    assert_eq!(
        one["locations"][0]["location"],
        "shop/pricing.py:41 in price_cart"
    );

    let (code, tl) = get(&format!("{base}/api/v1/timeline?since=2h&step=10m")).await;
    assert_eq!(code, 200, "{tl}");
    assert_eq!(tl["step_seconds"], 600.0);
    assert_eq!(tl["series"][0]["level"], "ERROR");
    let errors: i64 = tl["series"][0]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_i64().unwrap())
        .sum();
    assert_eq!(errors, 12);
    assert_eq!(tl["markers"][0]["name"], "v2.0.0");

    let (code, tl) = get(&format!("{base}/api/v1/timeline?since=2h&pattern_id={id}")).await;
    assert_eq!(code, 200, "{tl}");
    assert_eq!(tl["series"][0]["pattern_id"], id.as_str());
}

#[tokio::test]
async fn answers_the_agent_questions_as_data() {
    let base = serve(open_cfg()).await;

    let (code, wn) = get(&format!("{base}/api/v1/whats-new?window=1h&min_level=WARN")).await;
    assert_eq!(code, 200, "{wn}");
    assert_eq!(wn["counts"]["new"], 1);
    let f = &wn["findings"][0];
    assert_eq!(f["kind"], "new");
    assert_eq!(f["count_window"], 12);
    assert!(f["per_hour_window"].is_number());
    assert!(f["sample"]["text"].as_str().unwrap().contains("KeyError"));
    let id = f["pattern"]["id"].as_str().unwrap().to_string();

    let (code, v) = get(&format!("{base}/api/v1/patterns/{id}/verify?since=v2.0.0")).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["verdict"], "appeared_after");
    assert_eq!(v["split"]["marker"], "v2.0.0");
    assert_eq!(v["after"]["count"], 12);

    let (code, c) = get(&format!("{base}/api/v1/compare")).await;
    assert_eq!(code, 200, "{c}");
    assert_eq!(c["split"]["marker"], "v2.0.0");
    // The error and the INFO line logged with it both started after the deploy.
    assert_eq!(c["counts"]["new_after"], 2);
    assert_eq!(c["changes"][0]["pattern"]["level"], "ERROR");
    assert_eq!(c["changes"][0]["kind"], "new_after");

    let (code, ctx) = get(&format!(
        "{base}/api/v1/context?pattern_id={id}&before=3&after=0"
    ))
    .await;
    assert_eq!(code, 200, "{ctx}");
    assert_eq!(ctx["lines"].as_array().unwrap().len(), 4);
    assert_eq!(ctx["anchor"]["pattern_id"], id.as_str());

    let (code, rq) = get(&format!("{base}/api/v1/requests/badreq-0001")).await;
    assert_eq!(code, 200, "{rq}");
    assert_eq!(rq["events"], 24);
    assert_eq!(rq["lines"][0]["offset_ms"], 0.0);
}

#[tokio::test]
async fn errors_are_json_with_useful_codes() {
    let base = serve(open_cfg()).await;
    let (code, e) = get(&format!("{base}/api/v1/patterns/nope")).await;
    assert_eq!(code, 404);
    assert!(e["error"].as_str().unwrap().contains("Unknown pattern_id"));
    let (code, e) = get(&format!("{base}/api/v1/whats-new?window=soon")).await;
    assert_eq!(code, 400);
    assert!(e["error"].as_str().unwrap().contains("window"));
    let (code, e) = get(&format!("{base}/api/v1/patterns?limit=many")).await;
    assert_eq!(code, 400);
    assert!(e["error"].is_string());
    let (code, _) = get(&format!("{base}/api/v1/requests/unknown-req")).await;
    assert_eq!(code, 404);
    let (code, e) = get(&format!("{base}/api/v1/nothing")).await;
    assert_eq!(code, 404);
    assert!(e["error"].is_string());
    let (code, a) = get(&format!("{base}/api/v1/alerts")).await;
    assert_eq!(code, 200);
    assert_eq!(a["enabled"], false);
    let (code, _) = get(&format!("{base}/api/v1/alerts/stream")).await;
    assert_eq!(code, 404);
}

async fn post_marker(base: &str, origin: Option<&str>, auth: Option<&str>) -> (u16, Value) {
    let mut r = reqwest::Client::new()
        .post(format!("{base}/api/v1/markers"))
        .header("content-type", "application/json")
        .body(json!({"name": "fix-currency", "note": "use the cart currency"}).to_string());
    if let Some(o) = origin {
        r = r.header("origin", o);
    }
    if let Some(a) = auth {
        r = r.header("authorization", a);
    }
    let r = r.send().await.unwrap();
    let code = r.status().as_u16();
    let body = r.text().await.unwrap();
    (
        code,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

#[tokio::test]
async fn markers_can_be_added_from_the_same_origin_only() {
    let base = serve(open_cfg()).await;
    let (code, m) = post_marker(&base, None, None).await;
    assert_eq!(code, 201, "{m}");
    assert_eq!(m["name"], "fix-currency");
    assert_eq!(m["kind"], "manual");

    let (code, m) = post_marker(&base, Some(&base), None).await;
    assert_eq!(code, 201, "{m}");

    let (code, e) = post_marker(&base, Some("https://evil.example"), None).await;
    assert_eq!(code, 403);
    assert!(e["error"].as_str().unwrap().contains("--cors-origin"));

    // A form post can't send JSON, so it is refused.
    let r = reqwest::Client::new()
        .post(format!("{base}/api/v1/markers"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("name=x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 415);

    let (_, list) = get(&format!("{base}/api/v1/markers")).await;
    let names: Vec<&str> = list["markers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.iter().filter(|n| **n == "fix-currency").count(), 2);
    assert!(names.contains(&"v2.0.0"));
}

#[tokio::test]
async fn rejects_foreign_host_headers_without_a_token() {
    let base = serve(open_cfg()).await;
    let r = reqwest::Client::new()
        .get(format!("{base}/api/v1/status"))
        .header("host", "evil.example:7341")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);
}

#[tokio::test]
async fn token_and_cors() {
    let ui = "http://localhost:5173";
    let base = serve(HttpConfig {
        token: Some(TOKEN.into()),
        api: true,
        cors_origins: vec![ui.into()],
        ..Default::default()
    })
    .await;
    let c = reqwest::Client::new();
    let url = format!("{base}/api/v1/status");
    assert_eq!(c.get(&url).send().await.unwrap().status().as_u16(), 401);
    let r = c
        .get(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("origin", ui)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers()["access-control-allow-origin"], ui);

    // Preflights carry no credentials and still get an answer.
    let r = c
        .request(reqwest::Method::OPTIONS, &url)
        .header("origin", ui)
        .header("access-control-request-method", "GET")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 204);
    assert!(
        r.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .contains("authorization")
    );

    // Other origins get no CORS headers.
    let r = c
        .get(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none());

    let auth = format!("Bearer {TOKEN}");
    let (code, _) = post_marker(&base, Some(ui), Some(&auth)).await;
    assert_eq!(code, 201);
    let (code, _) = post_marker(&base, Some(ui), None).await;
    assert_eq!(code, 401);
}

#[tokio::test]
async fn api_can_be_turned_off() {
    let base = serve(HttpConfig::default()).await;
    let r = reqwest::get(format!("{base}/api/v1/status")).await.unwrap();
    assert_eq!(r.status().as_u16(), 404);
}

#[tokio::test]
async fn alert_stream_is_server_sent_events() {
    let store = seeded_store();
    let alerts = starlist::alerts::spawn_watcher(store.clone(), Default::default());
    let router = app(
        store,
        HttpConfig {
            api: true,
            alerts: Some(alerts),
            ..Default::default()
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    let r = reqwest::get(format!("http://{addr}/api/v1/alerts/stream"))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers()["content-type"], "text/event-stream");
    let (_, a) = get(&format!("http://{addr}/api/v1/alerts")).await;
    assert_eq!(a["enabled"], true);
}

async fn post_json(url: &str, body: Value) -> (u16, Value) {
    let r = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let code = r.status().as_u16();
    let text = r.text().await.unwrap();
    (
        code,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn issue_states_and_releases() {
    let base = serve(open_cfg()).await;
    let (_, ps) = get(&format!("{base}/api/v1/patterns?min_level=ERROR")).await;
    let id = ps["patterns"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(ps["patterns"][0]["state"]["state"], "open");

    let (code, p) = post_json(
        &format!("{base}/api/v1/patterns/{id}/state"),
        json!({"state": "ignored", "ignore_for": "2h", "note": "known"}),
    )
    .await;
    assert_eq!(code, 200, "{p}");
    assert_eq!(p["state"]["state"], "ignored");
    assert_eq!(p["state"]["note"], "known");
    assert!(p["state"]["until"].is_string());

    let (_, wn) = get(&format!("{base}/api/v1/whats-new?min_level=ERROR")).await;
    assert_eq!(wn["total"], 0);
    let (_, ign) = get(&format!("{base}/api/v1/patterns?state=ignored")).await;
    assert_eq!(ign["total"], 1);

    let (code, e) = post_json(
        &format!("{base}/api/v1/patterns/{id}/state"),
        json!({"state": "maybe"}),
    )
    .await;
    assert_eq!(code, 400, "{e}");

    let (code, rs) = get(&format!("{base}/api/v1/releases")).await;
    assert_eq!(code, 200);
    assert_eq!(rs["releases"][0]["name"], "v2.0.0");
    assert_eq!(rs["releases"][0]["current"], true);
    let (code, r) = get(&format!("{base}/api/v1/releases/current")).await;
    assert_eq!(code, 200, "{r}");
    assert_eq!(r["name"], "v2.0.0");
    assert_eq!(r["new"].as_array().unwrap().len(), 2);
    let (code, _) = get(&format!("{base}/api/v1/releases/v9")).await;
    assert_eq!(code, 404);

    let (_, all) = get(&format!("{base}/api/v1/patterns")).await;
    let ids: Vec<String> = all["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect();
    let (code, m) = post_json(
        &format!("{base}/api/v1/patterns/merge"),
        json!({"ids": [ids[1], ids[2]]}),
    )
    .await;
    assert_eq!(code, 200, "{m}");
    assert_eq!(m["id"], ids[1].as_str());
    let (_, all) = get(&format!("{base}/api/v1/patterns")).await;
    assert_eq!(all["total"], 2);
}
