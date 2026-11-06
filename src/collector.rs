//! The always-on part: reads log lines from files or stdin, groups them into
//! events, redacts, and records them in the store.

use crate::merge::Merger;
use crate::pattern::{Assembler, Event, parse_event};
use crate::redact::Redactor;
use crate::store::Store;
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Default rule for spotting deploys in logs, e.g. "Starting checkout v1.4.2"
/// or "server started, version=2.0.1".
pub const DEFAULT_MARKER_REGEX: &str =
    r"(?i)\bstart(?:ing|ed)?\b.*?\bv(?:ersion)?\s*[=:]?\s*v?(?P<name>\d+\.\d+(?:\.\d+)?[\w.+-]*)";

const BATCH: usize = 500;
const IDLE_FLUSH: Duration = Duration::from_millis(800);
const POLL: Duration = Duration::from_millis(250);

pub struct Pipeline {
    pub store: Arc<Store>,
    redactor: Redactor,
    marker_re: Option<Regex>,
    merger: Option<Mutex<Merger>>,
    /// Seconds east of UTC for log timestamps that have no zone.
    log_tz: i32,
}

impl Pipeline {
    pub fn new(store: Arc<Store>, redactor: Redactor, marker_re: Option<Regex>) -> Self {
        Self::with_merging(
            store,
            redactor,
            marker_re,
            Some(crate::merge::DEFAULT_THRESHOLD),
        )
    }

    /// `merge_after`: merge patterns differing in one word once that word has
    /// taken this many values; `None` turns merging off.
    pub fn with_merging(
        store: Arc<Store>,
        redactor: Redactor,
        marker_re: Option<Regex>,
        merge_after: Option<usize>,
    ) -> Self {
        let merger = merge_after.map(|t| {
            Mutex::new(Merger::load(t, &store).unwrap_or_else(|e| {
                eprintln!("starlist: could not load patterns for merging: {e}");
                Merger::new(t)
            }))
        });
        Self {
            store,
            redactor,
            marker_re,
            merger,
            log_tz: 0,
        }
    }

    /// Read zone-less log timestamps as this offset from UTC (seconds east).
    pub fn with_log_tz(mut self, offset_secs: i32) -> Self {
        self.log_tz = offset_secs;
        self
    }

    pub fn parse(&self, text: &str, now: f64) -> Event {
        let mut e = parse_event(text, &self.redactor, None, None);
        if e.ts_naive
            && let Some(t) = e.ts.as_mut()
        {
            *t -= self.log_tz as f64;
        }
        e.ts.get_or_insert(now);
        e
    }

    /// Parse, record, and pick up deploy markers from a batch of raw events.
    pub fn ingest(&self, texts: &[String]) {
        if texts.is_empty() {
            return;
        }
        let now = crate::analysis::now();
        let mut events: Vec<Event> = texts.iter().map(|t| self.parse(t, now)).collect();
        if let Some(m) = &self.merger {
            let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
            for e in events.iter_mut() {
                if let Some(op) = m.resolve(e) {
                    match self.store.merge_patterns(&op) {
                        Ok(()) => eprintln!(
                            "starlist: merged {} patterns into {}: {}",
                            op.aliases.len(),
                            op.canonical,
                            op.template
                        ),
                        Err(err) => eprintln!("starlist: merge failed: {err}"),
                    }
                }
            }
            // Earlier events in this batch may belong to a pattern that was just merged.
            for e in events.iter_mut() {
                if let Some(c) = m.canonical_of(&e.pattern_id).map(str::to_string) {
                    e.pattern_id = c;
                }
            }
        }
        if let Err(e) = self.store.record(&events) {
            eprintln!("starlist: failed to record events: {e}");
        }
        if let Some(re) = &self.marker_re {
            for e in &events {
                if let Some(name) = re.captures(&e.head).and_then(|c| c.name("name")) {
                    let name = format!("v{}", name.as_str().trim_start_matches('v'));
                    if let Ok(true) = self.store.add_deploy(&name, e.ts.unwrap(), None) {
                        eprintln!("starlist: deploy marker {name}");
                    }
                }
            }
        }
    }
}

/// Reads lines from any source, groups them into events and ingests in batches.
struct Feeder<'a> {
    pipeline: &'a Pipeline,
    asm: Assembler,
    batch: Vec<String>,
    last_line: Instant,
}

impl<'a> Feeder<'a> {
    fn new(pipeline: &'a Pipeline) -> Self {
        Self {
            pipeline,
            asm: Assembler::default(),
            batch: vec![],
            last_line: Instant::now(),
        }
    }
    fn line(&mut self, l: &str) {
        self.last_line = Instant::now();
        if let Some(ev) = self.asm.feed(l) {
            self.batch.push(ev);
            if self.batch.len() >= BATCH {
                self.commit();
            }
        }
    }
    /// Called when the source is idle: finish a pending event if nothing new arrived for a while.
    fn idle(&mut self, force: bool) {
        if self.asm.has_pending() && (force || self.last_line.elapsed() >= IDLE_FLUSH) {
            self.batch.extend(self.asm.flush());
        }
        self.commit();
    }
    fn commit(&mut self) {
        if !self.batch.is_empty() {
            self.pipeline.ingest(&std::mem::take(&mut self.batch));
        }
    }
}

#[cfg(unix)]
fn file_id(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.ino()
}
#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> u64 {
    0
}

/// Follow a file like `tail -F`: survives the file not existing yet, rotation and truncation.
/// With `once`, reads what is there and returns (for backfilling).
pub fn tail_file(
    pipeline: &Pipeline,
    path: PathBuf,
    from_start: bool,
    once: bool,
    stop: &AtomicBool,
) {
    let mut feeder = Feeder::new(pipeline);
    let mut first_open = true;
    'outer: while !stop.load(Ordering::Relaxed) {
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                if once {
                    eprintln!("starlist: cannot open {}: {e}", path.display());
                    return;
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let id = file.metadata().map(|m| file_id(&m)).unwrap_or(0);
        let mut reader = BufReader::new(file);
        let mut pos = if first_open && !from_start {
            reader.seek(SeekFrom::End(0)).unwrap_or(0)
        } else {
            0
        };
        first_open = false;
        let mut partial = String::new();
        loop {
            if stop.load(Ordering::Relaxed) {
                break 'outer;
            }
            let mut buf = String::new();
            match reader.read_line(&mut buf) {
                Ok(0) => {
                    if once {
                        if !partial.is_empty() {
                            feeder.line(&std::mem::take(&mut partial));
                        }
                        feeder.idle(true);
                        return;
                    }
                    feeder.idle(false);
                    // Rotated (new file at the path) or truncated?
                    match std::fs::metadata(&path) {
                        Ok(m) if file_id(&m) != id || m.len() < pos => {
                            feeder.idle(true);
                            continue 'outer;
                        }
                        Err(_) => {} // Moved away, new one not created yet: keep reading the old handle.
                        _ => {}
                    }
                    thread::sleep(POLL);
                }
                Ok(n) => {
                    pos += n as u64;
                    if buf.ends_with('\n') {
                        partial.push_str(&buf);
                        feeder.line(&std::mem::take(&mut partial));
                    } else {
                        partial.push_str(&buf); // Writer is mid-line; wait for the rest.
                    }
                }
                Err(e) => {
                    // Invalid UTF-8 and the like: skip the line.
                    eprintln!("starlist: read error in {}: {e}", path.display());
                    let mut skip = Vec::new();
                    let _ = reader.read_until(b'\n', &mut skip);
                    pos += skip.len() as u64;
                }
            }
        }
    }
    feeder.idle(true);
}

/// Read stdin to the end (e.g. `my-app | starlist collect --stdin`). With `tee`,
/// lines are passed through to stdout so the developer still sees their logs.
pub fn read_stdin(pipeline: &Pipeline, tee: bool) {
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut out = std::io::stdout();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if tee {
                let _ = writeln!(out, "{line}");
            }
            if tx.send(Some(line)).is_err() {
                return;
            }
        }
        let _ = tx.send(None);
    });
    let mut feeder = Feeder::new(pipeline);
    loop {
        match rx.recv_timeout(POLL) {
            Ok(Some(line)) => feeder.line(&line),
            Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => feeder.idle(false),
        }
    }
    feeder.idle(true);
}

/// Background thread that drops counts older than the retention period.
pub fn spawn_pruner(store: Arc<Store>, retention_secs: f64) {
    thread::spawn(move || {
        loop {
            let _ = store.prune(crate::analysis::now() - retention_secs);
            thread::sleep(Duration::from_secs(3600));
        }
    });
}
