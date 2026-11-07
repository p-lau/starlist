//! Follow logs by running a command-line tool (`docker logs -f`, `journalctl -f`,
//! `kubectl logs -f`) and reading its output, restarting it when it exits, and
//! following targets that come and go (containers with a label, the pods of a
//! deployment).

use super::{Resume, Start};
use crate::collector::{Feeder, POLL, Pipeline};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// What each output stream (stdout, stderr) has read so far. They are kept
/// apart because the two pipes can deliver lines out of order.
pub type Resumes = [Resume; 2];

/// How often targets are listed again.
pub const DISCOVER_EVERY: Duration = Duration::from_secs(5);

/// One source's command and how to read its output.
pub trait Tool: Send + Sync + 'static {
    /// Shown in messages, like "docker api".
    fn label(&self, target: &str) -> String;
    /// The command that follows `target` from `start`.
    fn command(&self, target: &str, start: Start) -> Command;
    /// A line of output: its time (Unix nanoseconds) and the log text, or `None` for a message
    /// from the tool itself (printed, not ingested). Text may hold several lines.
    fn decode(&self, line: &str, stderr: bool) -> Option<(i64, String)>;
}

/// Follow one target until `alive` turns false (or `stop` is set), restarting
/// the command when it exits and resuming where it left off.
pub fn follow(
    pipeline: &Pipeline,
    tool: &dyn Tool,
    target: &str,
    first: Start,
    resume: &Mutex<Resumes>,
    alive: &AtomicBool,
    stop: &AtomicBool,
) {
    let label = tool.label(target);
    let mut last_note = String::new();
    let mut backoff = Duration::from_secs(1);
    while alive.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        // Resume from the older of the two streams' newest lines; each stream
        // skips what it already read.
        let start = match lock(resume).iter().filter_map(|r| r.last).min() {
            Some(t) => Start::Since(t),
            None => first,
        };
        let mut cmd = tool.command(target, start);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("starlist: {label}: cannot run {:?}: {e}", cmd.get_program());
                return;
            }
        };
        let (tx, rx) = mpsc::channel::<(bool, String)>();
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|o| Box::new(o) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|e| Box::new(e) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .enumerate()
        .filter_map(|(i, r)| r.map(|r| (i == 1, r)))
        .map(|(is_err, r)| {
            let tx = tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(r).lines() {
                    let Ok(line) = line else { break };
                    if tx.send((is_err, line)).is_err() {
                        break;
                    }
                }
            })
        })
        .collect();
        drop(tx);
        // Docker passes a container's stdout and stderr on separately; keep
        // their multi-line events apart.
        let mut out = Feeder::new(pipeline);
        let mut err = Feeder::new(pipeline);
        let mut got_lines = false;
        loop {
            match rx.recv_timeout(POLL) {
                Ok((is_err, line)) => match tool.decode(&line, is_err) {
                    Some((ts, text)) => {
                        if !lock(resume)[is_err as usize].admit(ts, &text) {
                            continue;
                        }
                        got_lines = true;
                        let f = if is_err { &mut err } else { &mut out };
                        for l in text.lines() {
                            f.line(l);
                        }
                    }
                    None => {
                        let note = line.trim();
                        if !note.is_empty() && note != last_note {
                            eprintln!("starlist: {label}: {note}");
                            last_note = note.to_string();
                        }
                    }
                },
                Err(RecvTimeoutError::Timeout) => {
                    out.idle(false);
                    err.idle(false);
                    if !alive.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                        let _ = child.kill();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        out.idle(true);
        err.idle(true);
        let _ = child.wait();
        for r in readers {
            let _ = r.join();
        }
        // Quick exits (no such container yet, unit not found, API down) back
        // off up to 30 seconds; a follower that read lines retries soon.
        backoff = if got_lines {
            Duration::from_secs(1)
        } else {
            (backoff * 2).min(Duration::from_secs(30))
        };
        sleep_unless(backoff, alive, stop);
    }
}

/// Follow every target `list` returns, checking for new and gone ones every
/// `DISCOVER_EVERY`. Targets there at the first listing start at `first`;
/// targets that appear later are read from their beginning.
pub fn follow_all<T: Tool>(
    pipeline: Arc<Pipeline>,
    tool: Arc<T>,
    mut list: impl FnMut() -> Result<Vec<String>, String>,
    first: Start,
    stop: &'static AtomicBool,
) {
    struct Followed {
        alive: Arc<AtomicBool>,
        resume: Arc<Mutex<Resumes>>,
    }
    let mut known: HashMap<String, Followed> = HashMap::new();
    let mut first_listing = true;
    let mut last_err = String::new();
    while !stop.load(Ordering::Relaxed) {
        match list() {
            Ok(targets) => {
                last_err.clear();
                for t in &targets {
                    let running = known
                        .get(t)
                        .is_some_and(|f| f.alive.load(Ordering::Relaxed));
                    if running {
                        continue;
                    }
                    let start = if first_listing { first } else { Start::All };
                    let f = known.entry(t.clone()).or_insert_with(|| Followed {
                        alive: Arc::new(AtomicBool::new(true)),
                        resume: Arc::default(),
                    });
                    f.alive.store(true, Ordering::Relaxed);
                    let (p, tool, t, alive, resume) = (
                        pipeline.clone(),
                        tool.clone(),
                        t.clone(),
                        f.alive.clone(),
                        f.resume.clone(),
                    );
                    eprintln!("starlist: following {}", tool.label(&t));
                    thread::spawn(move || {
                        follow(&p, tool.as_ref(), &t, start, &resume, &alive, stop);
                        alive.store(false, Ordering::Relaxed);
                    });
                }
                for (t, f) in &known {
                    if !targets.contains(t) && f.alive.swap(false, Ordering::Relaxed) {
                        eprintln!("starlist: stopped following {} (gone)", tool.label(t));
                    }
                }
                first_listing = false;
            }
            Err(e) => {
                if e != last_err {
                    eprintln!("starlist: {e}");
                    last_err = e;
                }
            }
        }
        sleep_unless(DISCOVER_EVERY, &AtomicBool::new(true), stop);
    }
}

/// Run a listing command and return its non-empty output lines.
pub fn lines_of(mut cmd: Command) -> Result<Vec<String>, String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if !out.status.success() {
        return Err(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// The program to run: an environment override (like `STARLIST_DOCKER=podman`)
/// or the default name.
pub fn program(env: &str, default: &str) -> String {
    std::env::var(env)
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn sleep_unless(d: Duration, alive: &AtomicBool, stop: &AtomicBool) {
    let mut left = d;
    while !left.is_zero() && alive.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        let step = left.min(POLL);
        thread::sleep(step);
        left -= step;
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
