//! journald: follow a systemd unit through `journalctl -f -o json`.

use super::Start;
use super::command::{self, Entry, Tool};
use crate::collector::Pipeline;
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub struct Journald {
    pub program: String,
}

impl Default for Journald {
    fn default() -> Self {
        Self {
            program: command::program("STARLIST_JOURNALCTL", "journalctl"),
        }
    }
}

impl Tool for Journald {
    fn label(&self, unit: &str) -> String {
        format!("journald unit {unit}")
    }

    fn command(&self, unit: &str, start: Start) -> Command {
        let mut c = Command::new(&self.program);
        c.args(["--follow", "--output=json", "--unit", unit]);
        match start {
            Start::New => {
                c.arg("--lines=0");
            }
            Start::All => {
                c.arg("--lines=all");
            }
            Start::Since(t) => {
                // Whole seconds; the lines in between are skipped as already read.
                c.arg("--lines=all")
                    .arg(format!("--since=@{}", t.div_euclid(1_000_000_000)));
            }
        }
        c
    }

    fn decode(&self, line: &str, stderr: bool) -> Option<Entry> {
        if stderr {
            return None;
        }
        decode_entry(line)
    }
}

/// One `journalctl -o json` entry: its time, its message, and a head of that
/// time and, when the entry says so, a level. Entries at the default priority
/// (info) keep the level the app wrote, if any. journald stores each line an
/// app writes as its own entry, so the head is only put on lines that start an
/// event; a stack trace's frames stay with the line that logged it.
pub fn decode_entry(line: &str) -> Option<Entry> {
    let v: Value = serde_json::from_str(line).ok()?;
    let us: i64 = v.get("__REALTIME_TIMESTAMP")?.as_str()?.parse().ok()?;
    let ts = us.checked_mul(1000)?;
    let text = match v.get("MESSAGE")? {
        Value::String(s) => s.clone(),
        // Not valid UTF-8: journald sends the bytes.
        Value::Array(bytes) => {
            let b: Vec<u8> = bytes
                .iter()
                .filter_map(|x| x.as_u64().map(|n| n as u8))
                .collect();
            String::from_utf8_lossy(&b).into_owned()
        }
        _ => return None,
    };
    let level = match v.get("PRIORITY").and_then(Value::as_str) {
        Some("0" | "1" | "2") => Some("FATAL"),
        Some("3") => Some("ERROR"),
        Some("4") => Some("WARN"),
        Some("7") => Some("DEBUG"),
        _ => None,
    };
    let first = text.lines().next().unwrap_or_default();
    let mut head = format!("{} ", super::rfc3339(ts));
    if let Some(l) = level
        && crate::pattern::normalize_level(first.split_whitespace().next().unwrap_or("")).is_none()
    {
        head.push_str(l);
        head.push(' ');
    }
    Some(Entry {
        ts,
        head,
        text,
        stream: None,
    })
}

/// Follow a systemd unit, like `api.service`.
pub fn follow_unit(pipeline: Arc<Pipeline>, unit: String, first: Start, stop: &'static AtomicBool) {
    let tool = Journald::default();
    eprintln!("starlist: following {}", tool.label(&unit));
    let alive = AtomicBool::new(true);
    let resume = Default::default();
    command::follow(&pipeline, &tool, &unit, first, &resume, &alive, stop);
}
