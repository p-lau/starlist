//! Push alerts for new and spiking errors (issue #12).

use rmcp::model::{
    ProtocolVersion, ReadResourceRequestParams, ResourceUpdatedNotificationParam,
    ServerNotification, SubscribeRequestParams, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, ServiceExt};
use starlist::alerts::{AlertConfig, Detector, RESOURCE_URI, post_webhook, spawn_watcher};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::time::Duration;

const NOW: f64 = 1_800_000_000.0;

fn store() -> Arc<Store> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-alerts-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    Arc::new(Store::open(p).unwrap())
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

fn record(s: &Store, text: &str, ts: f64, n: usize) {
    let evs: Vec<Event> = (0..n).map(|i| ev(text, ts + i as f64 * 0.01)).collect();
    s.record(&evs).unwrap();
}

/// A day of steady, healthy traffic before `NOW`.
fn baseline(s: &Store) {
    let evs: Vec<Event> = (0..2400)
        .map(|i| ev("INFO GET /items 200", NOW - 86400.0 + i as f64 * 36.0))
        .collect();
    s.record(&evs).unwrap();
}

#[test]
fn one_incident_is_one_notification() {
    let s = store();
    baseline(&s);
    let mut d = Detector::new(AlertConfig::default());
    assert!(d.tick(&s, NOW).is_none(), "nothing is wrong yet");

    record(
        &s,
        "ERROR payment failed\nKeyError: 'currency'",
        NOW + 1.0,
        3,
    );
    assert!(
        d.tick(&s, NOW + 2.0).is_none(),
        "waits for the rest of the incident"
    );
    record(
        &s,
        "ERROR refund failed\nKeyError: 'currency'",
        NOW + 4.0,
        2,
    );
    record(&s, "INFO cache warmed", NOW + 4.0, 5);
    let a = d.tick(&s, NOW + 12.0).expect("alert after the debounce");
    assert_eq!(a.findings.len(), 2, "{a:?}");
    assert!(
        a.summary.starts_with("2 new or spiking error patterns"),
        "{}",
        a.summary
    );

    // The same errors keep happening: no repeat within the cooldown.
    record(
        &s,
        "ERROR payment failed\nKeyError: 'currency'",
        NOW + 30.0,
        5,
    );
    assert!(d.tick(&s, NOW + 50.0).is_none());

    // A different new error soon after waits for the minimum gap.
    record(&s, "FATAL worker crashed", NOW + 40.0, 1);
    assert!(
        d.tick(&s, NOW + 55.0).is_none(),
        "debounced, and within the 60s gap"
    );
    let b = d
        .tick(&s, NOW + 75.0)
        .expect("second incident after the gap");
    assert_eq!(b.findings.len(), 1);
    assert!(
        b.summary
            .starts_with("New error pattern: FATAL worker crashed"),
        "{}",
        b.summary
    );
}

#[test]
fn warnings_and_steady_errors_do_not_alert() {
    let s = store();
    baseline(&s);
    // An error that has always been there at the same rate.
    let evs: Vec<Event> = (0..240)
        .map(|i| ev("ERROR db retry", NOW - 86400.0 + i as f64 * 360.0))
        .collect();
    s.record(&evs).unwrap();
    record(&s, "WARN slow query", NOW + 1.0, 20);
    let mut d = Detector::new(AlertConfig::default());
    assert!(d.tick(&s, NOW + 2.0).is_none());
    assert!(d.tick(&s, NOW + 30.0).is_none());
}

#[test]
fn webhook_receives_the_alert_as_json() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(sock.try_clone().unwrap());
        let mut len = 0usize;
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        sock.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        (first, String::from_utf8(body).unwrap())
    });
    post_webhook(
        &url,
        &serde_json::json!({"source": "starlist", "summary": "New error pattern: boom"}),
    );
    let (first, body) = server.join().unwrap();
    assert!(first.starts_with("POST /hook"), "{first}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["summary"], "New error pattern: boom");
}

/// A client that forwards resources/updated notifications to a channel.
struct Listener(tokio::sync::mpsc::UnboundedSender<String>);

impl ClientHandler for Listener {
    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let _ = self.0.send(params.uri);
    }
}

/// Serves the MCP endpoint over HTTP against a store with a quiet day of
/// traffic, with a fast alert watcher unless `alerts` is false.
async fn serve(alerts: bool) -> anyhow::Result<(Arc<Store>, String)> {
    let s = store();
    let now = starlist::analysis::now();
    let evs: Vec<Event> = (0..600)
        .map(|i| ev("INFO GET /items 200", now - 86400.0 + i as f64 * 144.0))
        .collect();
    s.record(&evs)?;
    let alerts = alerts.then(|| {
        spawn_watcher(
            s.clone(),
            AlertConfig {
                interval: Duration::from_millis(200),
                debounce_secs: 0.5,
                ..Default::default()
            },
        )
    });
    let router = starlist::http::router(s.clone(), None, vec![], alerts);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, router).await });
    Ok((s, format!("http://{addr}/mcp")))
}

fn crash(s: &Store) -> anyhow::Result<f64> {
    let at = starlist::analysis::now();
    s.record(&[ev("FATAL worker crashed\nPanicError: out of memory", at)])?;
    Ok(at)
}

/// A client on the current protocol, which has no initialize handshake.
async fn modern_client(url: String) -> anyhow::Result<RunningService<RoleClient, ()>> {
    Ok(()
        .serve_with_lifecycle(
            StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(url),
            ),
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::LATEST],
            },
        )
        .await?)
}

fn whats_new_filter() -> SubscriptionFilter {
    SubscriptionFilter::builder()
        .resource_subscription(RESOURCE_URI)
        .build()
}

#[tokio::test]
async fn subscribed_agent_hears_about_a_new_crash_within_seconds() -> anyhow::Result<()> {
    let (s, url) = serve(true).await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client = Listener(tx)
        .serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url),
        ))
        .await?;
    let resources = client.list_resources(None).await?;
    assert_eq!(resources.resources[0].uri, RESOURCE_URI);
    #[allow(deprecated)]
    client
        .subscribe(SubscribeRequestParams::new(RESOURCE_URI))
        .await?;

    let at = crash(&s)?;

    let uri = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await?;
    assert_eq!(uri.as_deref(), Some(RESOURCE_URI));
    assert!(starlist::analysis::now() - at < 10.0);

    let r = client
        .read_resource(ReadResourceRequestParams::new(RESOURCE_URI))
        .await?;
    let text = serde_json::to_string(&r)?;
    assert!(
        text.contains("latest_alert") && text.contains("worker crashed"),
        "{text}"
    );
    Ok(())
}

#[tokio::test]
async fn listening_agent_hears_about_a_new_crash_within_seconds() -> anyhow::Result<()> {
    let (s, url) = serve(true).await?;
    let client = modern_client(url).await?;
    let mut sub = client.listen(whats_new_filter()).await?;
    assert_eq!(sub.acknowledged(), &whats_new_filter());

    let at = crash(&s)?;

    let n = tokio::time::timeout(Duration::from_secs(10), sub.next())
        .await??
        .expect("subscription ended early");
    match n {
        ServerNotification::ResourceUpdatedNotification(n) => {
            assert_eq!(n.params.uri, RESOURCE_URI)
        }
        other => panic!("unexpected notification: {other:?}"),
    }
    assert!(starlist::analysis::now() - at < 10.0);

    let r = client
        .read_resource(ReadResourceRequestParams::new(RESOURCE_URI))
        .await?;
    assert!(serde_json::to_string(&r)?.contains("worker crashed"));

    // Ending the subscription leaves the session usable.
    sub.cancel().await?;
    assert_eq!(
        client.list_resources(None).await?.resources[0].uri,
        RESOURCE_URI
    );
    Ok(())
}

#[tokio::test]
async fn listen_is_refused_when_alerts_are_off() -> anyhow::Result<()> {
    let (_s, url) = serve(false).await?;
    let client = modern_client(url).await?;
    let err = client
        .listen(whats_new_filter())
        .await
        .expect_err("listen should be refused without alerts");
    assert!(err.to_string().contains("-32601"), "{err}");
    // The session keeps working for everything else.
    client.list_resources(None).await?;
    Ok(())
}
