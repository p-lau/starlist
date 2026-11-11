//! End-to-end tests of the starlist-ci binary.

use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const PASSING: &str = "\
2026-11-10T10:00:00.0000000Z ##[group]Run cargo test
2026-11-10T10:00:01.0000000Z    Compiling app v0.1.0 (/home/runner/work/app/app)
2026-11-10T10:00:02.0000000Z WARN cache miss for key k1
2026-11-10T10:00:02.0000000Z WARN retrying request 1 of 3
2026-11-10T10:00:03.0000000Z INFO connected to db in 12ms
2026-11-10T10:00:04.0000000Z INFO seeding fixtures done
2026-11-10T10:00:05.0000000Z test result: ok. 12 passed; 0 failed; 0 ignored
";

const FAILING: &str = "\
2026-11-11T10:00:00.0000000Z ##[group]Run cargo test
2026-11-11T10:00:01.0000000Z    Compiling app v0.1.1 (/home/runner/work/app/app)
2026-11-11T10:00:02.0000000Z WARN cache miss for key k9
2026-11-11T10:00:02.0000000Z WARN retrying request 1 of 3
2026-11-11T10:00:02.0000000Z WARN retrying request 2 of 3
2026-11-11T10:00:02.0000000Z WARN retrying request 3 of 3
2026-11-11T10:00:03.0000000Z INFO connected to db in 950ms
2026-11-11T10:00:04.0000000Z ERROR payment failed for order 42 password=hunter2
2026-11-11T10:00:04.0000000Z Traceback (most recent call last):
2026-11-11T10:00:04.0000000Z   File \"/home/runner/work/app/app/billing/charge.py\", line 88, in charge
2026-11-11T10:00:04.0000000Z ValueError: card declined
2026-11-11T10:00:05.0000000Z test result: FAILED. 11 passed; 1 failed; 0 ignored
2026-11-11T10:00:05.0000000Z ##[error]Process completed with exit code 101.
";

/// A fresh, empty directory for one test.
fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("starlist-ci-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &PathBuf, args: &[&str], stdin: Option<&str>) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_starlist-ci"))
        .args(args)
        .current_dir(dir)
        .env_remove("GITHUB_WORKSPACE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut i = child.stdin.take().unwrap();
        if let Some(s) = stdin {
            i.write_all(s.as_bytes()).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn logs(name: &str) -> PathBuf {
    let d = tempdir(name);
    std::fs::write(d.join("pass.log"), PASSING).unwrap();
    std::fs::write(d.join("fail.log"), FAILING).unwrap();
    d
}

fn finding<'a>(v: &'a Value, needle: &str) -> &'a Value {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["template"].as_str().unwrap().contains(needle))
        .unwrap_or_else(|| panic!("no finding with {needle}: {v:#}"))
}

#[test]
fn reports_new_grown_and_stopped_patterns() {
    let d = logs("compare");
    let args = [
        "--baseline",
        "pass.log",
        "fail.log",
        "--source-root",
        "/home/runner/work/app/app",
    ];
    let v: Value = serde_json::from_str(&run(&d, &args, None)).unwrap();

    // The error comes first, with its code location, line and step.
    let first = &v["findings"][0];
    assert_eq!(first["kind"], "new");
    assert_eq!(first["level"], "ERROR");
    assert!(
        first["template"]
            .as_str()
            .unwrap()
            .contains("payment failed")
    );
    assert_eq!(first["location"], "billing/charge.py:88 in charge");
    assert_eq!(first["line"], 8);
    assert_eq!(first["step"], "Run cargo test");
    // Redacted before anything is shown.
    assert!(!v.to_string().contains("hunter2"), "{v:#}");
    assert!(first["sample"].as_str().unwrap().contains("ValueError"));

    let retry = finding(&v, "retrying request");
    assert_eq!(retry["kind"], "grew");
    assert_eq!(
        (retry["count"].as_u64(), retry["before"].as_u64()),
        (Some(3), Some(1))
    );

    assert_eq!(finding(&v, "test result: ok")["kind"], "stopped");
    assert_eq!(finding(&v, "test result: FAILED")["level"], "ERROR?");
    assert_eq!(finding(&v, "exit code")["kind"], "new");

    // Same shape, different values: not a change.
    let all = v["findings"].to_string();
    assert!(!all.contains("cache miss"), "{v:#}");
    assert!(!all.contains("connected to db"), "{v:#}");
    assert!(!all.contains("Compiling"), "{v:#}");

    // Nothing left behind but the two logs.
    let mut files: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["fail.log", "pass.log"]);
}

#[test]
fn reads_stdin_and_lists_errors_without_a_baseline() {
    let d = tempdir("stdin");
    let v: Value = serde_json::from_str(&run(&d, &[], Some(FAILING))).unwrap();
    assert!(v["summary"].as_str().unwrap().starts_with("No passing run"));
    assert!(v["events"].get("passing").is_none());
    let findings = v["findings"].as_array().unwrap();
    // Errors and warnings only, with no comparison fields.
    assert!(findings.iter().all(|f| f.get("kind").is_none()));
    assert!(findings.iter().any(|f| f["level"] == "WARN"));
    assert!(
        !findings
            .iter()
            .any(|f| f["template"].as_str().unwrap().contains("connected to db"))
    );
}

#[test]
fn markdown_for_people() {
    let d = logs("markdown");
    let md = run(
        &d,
        &["-b", "pass.log", "fail.log", "--format", "markdown"],
        None,
    );
    assert!(md.starts_with("### starlist-ci"), "{md}");
    assert!(md.contains("| new | ERROR |"), "{md}");
    assert!(md.contains("<details>"), "{md}");
    // Masks are escaped where Markdown would read them as HTML.
    let summaries: Vec<&str> = md.lines().filter(|l| l.contains("<summary>")).collect();
    assert!(summaries.iter().all(|l| !l.contains("<NUM>")), "{md}");
}

#[test]
fn stays_within_the_size_limit() {
    let d = tempdir("budget");
    let failing: String = (0..200)
        .map(|i| {
            format!(
                "ERROR job {} broke: {}\n  at handler (/srv/app/src/h{i}.js:{i}:1)\n",
                ["alpha", "beta", "gamma"][i % 3],
                "x".repeat(i % 7 + 1) + &"word ".repeat(i % 11 + 1)
            )
        })
        .collect();
    std::fs::write(d.join("fail.log"), failing).unwrap();
    std::fs::write(d.join("pass.log"), "INFO all good\n").unwrap();
    let out = run(
        &d,
        &[
            "-b",
            "pass.log",
            "fail.log",
            "--max-chars",
            "1500",
            "--detail",
            "full",
        ],
        None,
    );
    assert!(out.trim_end().chars().count() <= 1500, "{}", out.len());
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(v["remaining"].as_u64().unwrap() > 0);
    assert!(v["trimmed"].as_str().unwrap().contains("max_chars"));
}
