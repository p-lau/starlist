//! Docker, journald and Kubernetes sources (issue #27), driven by fake
//! command-line tools.

use starlist::collector::Pipeline;
use starlist::redact::Redactor;
use starlist::sources::command::{self, Tool};
use starlist::sources::journald::decode_entry;
use starlist::sources::kubernetes::{Kubectl, selector_from_labels};
use starlist::sources::{Resume, Start, split_timestamp};
use starlist::store::Store;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn store(name: &str) -> Arc<Store> {
    let p = std::env::temp_dir().join(format!("starlist-src-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    Arc::new(Store::open(p).unwrap())
}

#[test]
fn timestamps_from_docker_and_kubectl() {
    let (t, rest) = split_timestamp("2026-11-07T01:02:03.5Z ERROR boom").unwrap();
    assert_eq!(rest, "ERROR boom");
    assert_eq!(
        starlist::sources::rfc3339(t),
        "2026-11-07T01:02:03.500000000Z"
    );
    let (t, _) = split_timestamp("2026-11-07T01:02:03.000000001Z x").unwrap();
    assert_eq!(
        starlist::sources::rfc3339(t),
        "2026-11-07T01:02:03.000000001Z"
    );
    assert!(split_timestamp("Error response from daemon: No such container").is_none());
}

#[test]
fn resuming_skips_lines_already_read() {
    let mut r = Resume::default();
    assert!(r.admit(10, "a"));
    assert!(r.admit(10, "b"));
    assert!(r.admit(11, "c"));
    // Read again after a restart from 11.
    assert!(!r.admit(10, "a"));
    assert!(!r.admit(11, "c"));
    assert!(r.admit(11, "d"));
    assert!(r.admit(12, "a"));
}

#[test]
fn journald_entries() {
    let e =
        r#"{"__REALTIME_TIMESTAMP":"1762477323500000","PRIORITY":"3","MESSAGE":"payment failed"}"#;
    let (ts, text) = decode_entry(e).unwrap();
    assert_eq!(ts, 1_762_477_323_500_000_000);
    assert_eq!(text, "2025-11-07T01:02:03.500000000Z ERROR payment failed");
    // The default priority keeps the app's own level.
    let e = r#"{"__REALTIME_TIMESTAMP":"1762477323500000","PRIORITY":"6","MESSAGE":"WARN disk at 91%"}"#;
    assert!(decode_entry(e).unwrap().1.ends_with("Z WARN disk at 91%"));
    // An app level wins over the priority, too.
    let e = r#"{"__REALTIME_TIMESTAMP":"1762477323500000","PRIORITY":"3","MESSAGE":"FATAL out of memory"}"#;
    assert!(
        decode_entry(e)
            .unwrap()
            .1
            .ends_with("Z FATAL out of memory")
    );
    // Not UTF-8: journald sends bytes.
    let e = r#"{"__REALTIME_TIMESTAMP":"1762477323500000","MESSAGE":[104,105,255]}"#;
    assert!(decode_entry(e).unwrap().1.ends_with("Z hi\u{fffd}"));
    assert!(decode_entry("-- No entries --").is_none());
}

#[test]
fn kubernetes_selectors_and_commands() {
    assert_eq!(
        selector_from_labels(r#"{"app":"api","tier":"web"}"#).as_deref(),
        Some("app=api,tier=web")
    );
    assert_eq!(selector_from_labels("{}"), None);
    let k = Kubectl {
        program: "kubectl".into(),
        namespace: Some("shop".into()),
    };
    assert_eq!(k.selector("app=api").unwrap(), "app=api");
    let args = |s| {
        k.command("api-7d9f", s)
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert!(args(Start::New).starts_with("--namespace shop logs --follow --timestamps"));
    assert!(args(Start::New).ends_with("--tail=0 api-7d9f"));
    assert!(
        args(Start::Since(1_500_000_000)).contains("--since-time=1970-01-01T00:00:01.500000000Z")
    );
}

/// A fake `docker`: each run prints the same lines (with a stack trace, on
/// stdout and stderr, plus a message from docker itself) and exits, so the
/// follower restarts it and must not read anything twice. The second run adds
/// a line.
#[cfg(unix)]
#[test]
fn docker_follower_restarts_without_reading_twice() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("starlist-fake-docker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let args_file = dir.join("args");
    let script = dir.join("docker");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
echo "$@" >> "{args}"
echo "2026-11-07T01:00:00.000000001Z 2026-11-07 01:00:00 ERROR checkout failed order=1"
echo "2026-11-07T01:00:00.000000002Z Traceback (most recent call last):"
echo '2026-11-07T01:00:00.000000003Z   File "shop/app.py", line 3, in main'
echo "2026-11-07T01:00:00.000000004Z KeyError: 'currency'"
echo "2026-11-07T01:00:01.000000000Z 2026-11-07 01:00:01 WARN slow upstream" >&2
echo "Error response from daemon: container is restarting" >&2
if [ "$(wc -l < "{args}")" -ge 2 ]; then
  echo "2026-11-07T01:00:02.000000000Z 2026-11-07 01:00:02 INFO back up"
fi
"#,
            args = args_file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let s = store("docker");
    let p = Pipeline::new(s.clone(), Redactor::default(), None);
    let tool = starlist::sources::docker::Docker {
        program: script.display().to_string(),
    };
    let alive = Arc::new(AtomicBool::new(true));
    let stop = AtomicBool::new(false);
    let a = alive.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2600));
        a.store(false, Ordering::Relaxed);
    });
    let resume = Default::default();
    command::follow(&p, &tool, "shop", Start::New, &resume, &alive, &stop);

    let args = std::fs::read_to_string(&args_file).unwrap();
    let runs: Vec<&str> = args.lines().collect();
    assert!(runs.len() >= 2, "{args}");
    assert_eq!(runs[0], "logs --follow --timestamps --tail 0 shop");
    // Resumes from the older stream's newest line.
    assert_eq!(
        runs[1],
        "logs --follow --timestamps --since 2026-11-07T01:00:00.000000004Z shop"
    );

    let patterns = s.patterns_seen_since(0.0).unwrap();
    let mut seen: Vec<(String, i64)> = patterns
        .iter()
        .map(|p| (p.level.clone().unwrap_or_default(), p.total))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [("ERROR".into(), 1), ("INFO".into(), 1), ("WARN".into(), 1)],
        "{patterns:?}"
    );
    let err = patterns
        .iter()
        .find(|p| p.level.as_deref() == Some("ERROR"))
        .unwrap();
    assert_eq!(err.exc_type.as_deref(), Some("KeyError"));
    assert_eq!(err.location.as_deref(), Some("shop/app.py:3 in main"));
}
