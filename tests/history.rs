//! Long-term history and metrics read from logs (issue #24).

use regex::Regex;
use serde_json::Value;
use starlist::collector::Pipeline;
use starlist::http::{HttpConfig, app};
use starlist::metrics::{Fields, summaries};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

const H: f64 = 3600.0;
const D: f64 = 86400.0;

fn store() -> Arc<Store> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-history-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&p);
    Arc::new(Store::open(p).unwrap())
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

fn pipeline(s: &Arc<Store>) -> Pipeline {
    Pipeline::new(s.clone(), Redactor::default(), None::<Regex>)
        .with_metrics(Fields::new(&["duration_ms".into()]).unwrap())
}

#[test]
fn rolled_up_counts_answer_the_same_questions() {
    let s = store();
    let now = starlist::analysis::now();
    // One error every 30 minutes for 20 days.
    let evs: Vec<Event> = (0..20 * 48)
        .map(|i| {
            ev(
                "ERROR sync failed: upstream 502",
                now - 20.0 * D + i as f64 * 1800.0,
            )
        })
        .collect();
    s.record(&evs).unwrap();
    let id = evs[0].pattern_id.clone();
    let before_month = s.count(&id, now - 30.0 * D, now).unwrap();
    let before_week = s.count(&id, now - 10.0 * D, now - 3.0 * D).unwrap();
    let series_before = s.histogram(&id, now - 21.0 * D, now, D).unwrap();
    assert_eq!(before_month, 960);

    // Keep 10-second counts for 7 days; everything older goes hourly.
    let folded = s.rollup(now - 7.0 * D, Some(now - 400.0 * D)).unwrap();
    assert!(folded > 600, "{folded}");
    assert_eq!(s.count(&id, now - 30.0 * D, now).unwrap(), before_month);
    // Older counts are per hour, so a range that starts mid-hour is read to the hour.
    let week = s.count(&id, now - 10.0 * D, now - 3.0 * D).unwrap();
    assert!((week - before_week).abs() <= 2, "{week} vs {before_week}");
    // Daily points line up the same, give or take an event at a day's edge.
    let series_after = s.histogram(&id, now - 21.0 * D, now, D).unwrap();
    assert_eq!(
        series_after.iter().sum::<i64>(),
        series_before.iter().sum::<i64>()
    );
    for (a, b) in series_after.iter().zip(&series_before) {
        assert!((a - b).abs() <= 2, "{series_after:?} vs {series_before:?}");
    }
    // Rolling up again changes nothing.
    assert_eq!(s.rollup(now - 7.0 * D, None).unwrap(), 0);
    assert_eq!(s.count(&id, now - 30.0 * D, now).unwrap(), before_month);

    // History older than the history period is dropped.
    s.rollup(now - 7.0 * D, Some(now - 14.0 * D)).unwrap();
    let left = s.count(&id, now - 30.0 * D, now).unwrap();
    assert!((14 * 48 - 2..=14 * 48 + 2).contains(&left), "{left}");

    // Resolving still works across the boundary: occurrences in rolled-up
    // hours are found at the hour.
    assert!(s.first_after(&id, now - 13.0 * D).unwrap().is_some());
}

#[test]
fn numeric_fields_become_hourly_percentiles() {
    let s = store();
    let p = pipeline(&s);
    let now = starlist::analysis::now();
    let mut evs = vec![];
    for i in 0..1000 {
        let ms = 10 + (i % 100) * 5; // 10 to 505
        evs.push(p.parse(
            &format!("INFO GET /cart 200 duration_ms={ms}"),
            now - 7200.0 + i as f64 * 7.0,
        ));
    }
    evs.push(p.parse(
        r#"{"level":"info","msg":"GET /cart 200","duration_ms": 9000}"#,
        now - 60.0,
    ));
    for e in &mut evs {
        e.ts.get_or_insert(now);
    }
    assert_eq!(evs[0].metrics, vec![("duration_ms".to_string(), 10.0)]);
    s.record(&evs).unwrap();
    let id = evs[0].pattern_id.clone();

    let sums = summaries(&s, Some(&id), now - D, now).unwrap();
    assert_eq!(sums.len(), 1);
    let f = &sums[0];
    assert_eq!(f.field, "duration_ms");
    assert_eq!(f.count, 1000);
    assert!((f.p50 - 257.0).abs() / 257.0 < 0.1, "{f:?}");
    assert!((f.p99 - 500.0).abs() / 500.0 < 0.1, "{f:?}");
    assert_eq!(f.min, 10.0);
    assert_eq!(f.max, 505.0);
    // Across patterns, the JSON line counts too.
    let all = summaries(&s, None, now - D, now).unwrap();
    assert_eq!(all[0].count, 1001);
    assert_eq!(all[0].max, 9000.0);

    // A merge moves the histograms with the counts.
    let other = p.parse("INFO GET /checkout 200 duration_ms=40", now - 30.0);
    let mut other = other;
    other.ts = Some(now - 30.0);
    s.record(std::slice::from_ref(&other)).unwrap();
    starlist::issues::merge(&s, &[id.clone(), other.pattern_id.clone()]).unwrap();
    let merged = summaries(&s, Some(&id), now - D, now).unwrap();
    assert_eq!(merged[0].count, 1001);
    assert_eq!(merged[0].min, 10.0);

    // Recording the same hour again adds to it.
    let mut more = p.parse("INFO GET /cart 200 duration_ms=1", now - 5.0);
    more.ts = Some(now - 5.0);
    s.record(&[more]).unwrap();
    assert_eq!(summaries(&s, Some(&id), now - D, now).unwrap()[0].min, 1.0);

    // MCP's get_pattern shows the summary.
    let v = starlist::analysis::get_pattern(
        &s,
        &id,
        None,
        8000,
        starlist::analysis::Detail::Brief,
        now,
    )
    .unwrap();
    let f = v["fields_last_24h"][0].as_str().unwrap();
    assert!(f.starts_with("duration_ms: p50"), "{v}");
}

#[test]
fn prometheus_and_export() {
    let s = store();
    let p = pipeline(&s);
    let now = starlist::analysis::now();
    // Keep the 20 lines inside one hour, or the export has two field rows.
    let hour = |t: f64| (t / 3600.0).floor();
    let start = if hour(now - 600.0) == hour(now - 581.0) {
        now - 600.0
    } else {
        now - 660.0
    };
    let mut evs = vec![];
    for i in 0..20 {
        let mut e = p.parse(&format!("INFO served duration_ms={}", 100 + i), start);
        e.ts = Some(start + i as f64);
        evs.push(e);
    }
    let mut err = p.parse("ERROR \"quoted\" failure", now - 30.0);
    err.ts = Some(now - 30.0);
    evs.push(err);
    s.record(&evs).unwrap();
    s.add_deploy("v3", now - 900.0, None).unwrap();
    starlist::issues::set_state(
        &s,
        &evs[20].pattern_id,
        "resolved",
        None,
        None,
        None,
        now - 60.0,
    )
    .unwrap();

    #[cfg(feature = "prometheus")]
    {
        let text = starlist::prometheus::render(&s, now).unwrap();
        assert!(
            text.contains("starlist_events_total{level=\"INFO\"} 20"),
            "{text}"
        );
        assert!(
            text.contains("starlist_events_total{level=\"ERROR\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("starlist_patterns_by_state{state=\"regressed\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("starlist_patterns_by_state{state=\"open\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("starlist_field{field=\"duration_ms\",quantile=\"0.5\"}"),
            "{text}"
        );
        assert!(
            text.contains("starlist_field_count{field=\"duration_ms\"} 20"),
            "{text}"
        );
        assert!(text.contains("# TYPE starlist_db_bytes gauge"));
        for l in text.lines().filter(|l| !l.starts_with('#')) {
            let (_, v) = l.rsplit_once(' ').unwrap();
            v.parse::<f64>()
                .unwrap_or_else(|_| panic!("bad sample line {l}"));
        }
    }

    let mut buf = vec![];
    let n = starlist::export::write(&s, now - D, &mut buf).unwrap();
    let lines: Vec<Value> = String::from_utf8(buf)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len() as u64, n);
    let of = |t: &str| lines.iter().filter(|l| l["type"] == t).count();
    assert_eq!(of("pattern"), 2);
    assert!(of("count") >= 2);
    assert_eq!(of("field"), 1);
    assert_eq!(of("marker"), 1);
    let counted: i64 = lines
        .iter()
        .filter(|l| l["type"] == "count")
        .map(|l| l["n"].as_i64().unwrap())
        .sum();
    assert_eq!(counted, 21);
    let state = lines
        .iter()
        .find(|l| l["type"] == "pattern" && l["level"] == "ERROR")
        .unwrap();
    assert_eq!(state["state"], "regressed");
}

async fn serve(s: Arc<Store>, cfg: HttpConfig) -> String {
    let router = app(s, cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{addr}")
}

#[tokio::test]
async fn fields_api_and_metrics_endpoint() {
    let s = store();
    let p = pipeline(&s);
    let now = starlist::analysis::now();
    let evs: Vec<Event> = (0..48)
        .map(|i| {
            let mut e = p.parse(&format!("INFO GET /a 200 duration_ms={}", 50 + i), now);
            e.ts = Some(now - 47.0 * H + i as f64 * H);
            e
        })
        .collect();
    s.record(&evs).unwrap();
    let id = evs[0].pattern_id.clone();

    let base = serve(
        s.clone(),
        HttpConfig {
            api: true,
            prometheus: true,
            ..Default::default()
        },
    )
    .await;
    let j = |u: String| async move {
        let r = reqwest::get(u).await.unwrap();
        (r.status().as_u16(), r.json::<Value>().await.unwrap())
    };
    let (c, v) = j(format!("{base}/api/v1/fields?since=3d")).await;
    assert_eq!(c, 200);
    assert_eq!(v["fields"][0]["field"], "duration_ms");
    assert_eq!(v["fields"][0]["count"], 48);

    let (c, v) = j(format!(
        "{base}/api/v1/fields/duration_ms?since=2d&step=6h&pattern_id={id}"
    ))
    .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(v["step_seconds"], 21600.0);
    let pts = v["points"].as_array().unwrap();
    let total: u64 = pts.iter().map(|p| p["count"].as_u64().unwrap()).sum();
    assert_eq!(total, 48);
    assert_eq!(v["summary"]["max"], 97.0);

    let (c, v) = j(format!("{base}/api/v1/patterns/{id}")).await;
    assert_eq!(c, 200);
    assert_eq!(v["fields"][0]["field"], "duration_ms");

    let (c, _) = j(format!("{base}/api/v1/fields/x?pattern_id=nope")).await;
    assert_eq!(c, 404);

    #[cfg(feature = "prometheus")]
    {
        let r = reqwest::get(format!("{base}/metrics")).await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        assert!(
            r.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/plain; version=0.0.4")
        );
        assert!(r.text().await.unwrap().contains("starlist_events_total"));

        // Off unless asked for, and behind the token like the API.
        let off = serve(
            s.clone(),
            HttpConfig {
                api: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            reqwest::get(format!("{off}/metrics"))
                .await
                .unwrap()
                .status()
                .as_u16(),
            404
        );
        let token = "0123456789abcdef0123456789abcdef";
        let locked = serve(
            s,
            HttpConfig {
                token: Some(token.into()),
                prometheus: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            reqwest::get(format!("{locked}/metrics"))
                .await
                .unwrap()
                .status()
                .as_u16(),
            401
        );
        let ok = reqwest::Client::new()
            .get(format!("{locked}/metrics"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status().as_u16(), 200);
    }
}
