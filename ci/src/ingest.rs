//! Reading one run's log into per-pattern counts, in memory.

use crate::clean::{Cleaner, Line};
use starlist_core::location::strip_root;
use starlist_core::merge::Merger;
use starlist_core::pattern::{Assembler, Event, level_rank, parse_event};
use starlist_core::redact::Redactor;
use std::collections::HashMap;
use std::io::BufRead;

/// What one run's log says about one pattern.
#[derive(Debug, Clone)]
pub struct Seen {
    pub template: String,
    pub level: Option<&'static str>,
    pub count: usize,
    /// Line number (from 1) where the first event of this pattern starts.
    pub line: usize,
    /// The step or group of the first event.
    pub section: Option<String>,
    /// The first event, redacted, with its stack trace.
    pub sample: String,
    pub location: Option<String>,
}

/// One run's log, distilled.
#[derive(Debug, Default)]
pub struct RunLog {
    pub patterns: HashMap<String, Seen>,
    pub events: usize,
    pub lines: usize,
}

/// Settings shared by both runs, so their pattern IDs line up.
pub struct Engine {
    pub redactor: Redactor,
    pub source_roots: Vec<String>,
    pub merger: Merger,
}

impl Engine {
    pub fn new(redactor: Redactor, source_roots: Vec<String>, merge_threshold: usize) -> Self {
        Self {
            redactor,
            source_roots,
            merger: Merger::new(merge_threshold),
        }
    }

    /// Read a whole log. Invalid UTF-8 is replaced rather than rejected.
    pub fn read(&mut self, mut input: impl BufRead) -> std::io::Result<RunLog> {
        let mut run = RunLog::default();
        let mut cleaner = Cleaner::default();
        let mut asm = Assembler::default();
        // Where the event being assembled started: line number and section.
        let mut start: (usize, Option<String>) = (0, None);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            if input.read_until(b'\n', &mut buf)? == 0 {
                break;
            }
            run.lines += 1;
            let raw = String::from_utf8_lossy(&buf);
            let text = match cleaner.clean(&raw) {
                Line::Text(t) => t,
                Line::Section(_) | Line::Skip => continue,
            };
            let had = asm.has_pending();
            let done = asm.feed(&text);
            let started = done.is_some() || (!had && asm.has_pending());
            if let Some(ev) = done {
                self.add(&mut run, &ev, &start);
            }
            if started {
                start = (run.lines, cleaner.section().map(str::to_string));
            }
        }
        if let Some(ev) = asm.flush() {
            self.add(&mut run, &ev, &start);
        }
        Ok(run)
    }

    fn add(&mut self, run: &mut RunLog, text: &str, start: &(usize, Option<String>)) {
        let mut e: Event = parse_event(text, &self.redactor, None, None);
        if let Some(l) = e.location.take() {
            e.location = Some(strip_root(&l, &self.source_roots).to_string());
        }
        // Merges are settled once both runs are read (see `settle`).
        let _ = self.merger.resolve(&mut e);
        run.events += 1;
        run.patterns
            .entry(e.pattern_id)
            .and_modify(|s| s.count += 1)
            .or_insert_with(|| Seen {
                template: e.template,
                level: e.level,
                count: 1,
                line: start.0,
                section: start.1.clone(),
                sample: e.body,
                location: e.location,
            });
    }

    /// Fold patterns that were merged after their first events into their
    /// canonical pattern, and give each its final template.
    pub fn settle(&self, run: &mut RunLog) {
        let mut out: HashMap<String, Seen> = HashMap::with_capacity(run.patterns.len());
        for (id, s) in run.patterns.drain() {
            let id = self.merger.canonical_of(&id).unwrap_or(&id).to_string();
            match out.get_mut(&id) {
                None => {
                    out.insert(id, s);
                }
                Some(o) => {
                    o.count += s.count;
                    if level_rank(s.level) > level_rank(o.level) {
                        o.level = s.level;
                    }
                    if s.line < o.line {
                        o.line = s.line;
                        o.section = s.section;
                        o.sample = s.sample;
                        o.location = s.location.or(o.location.take());
                    } else if o.location.is_none() {
                        o.location = s.location;
                    }
                }
            }
        }
        for (id, s) in out.iter_mut() {
            if let Some(t) = self.merger.template_of(id) {
                t.clone_into(&mut s.template);
            }
        }
        run.patterns = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(engine: &mut Engine, log: &str) -> RunLog {
        let mut r = engine.read(log.as_bytes()).unwrap();
        engine.settle(&mut r);
        r
    }

    #[test]
    fn counts_events_with_their_first_line_and_step() {
        let mut e = Engine::new(Redactor::default(), vec![], 5);
        let log = "##[group]Run tests\nINFO start 1\nINFO start 2\nERROR boom\nTraceback (most recent call last):\n  File \"/w/app/x.py\", line 3, in f\nValueError: bad\n";
        let r = read(&mut e, log);
        assert_eq!(r.events, 3);
        let start = r
            .patterns
            .values()
            .find(|s| s.template.contains("start"))
            .unwrap();
        assert_eq!((start.count, start.line), (2, 2));
        assert_eq!(start.section.as_deref(), Some("Run tests"));
        let boom = r
            .patterns
            .values()
            .find(|s| s.template.contains("boom"))
            .unwrap();
        assert_eq!(boom.line, 4);
    }

    #[test]
    fn merged_patterns_are_folded() {
        let mut e = Engine::new(Redactor::default(), vec![], 5);
        let log: String = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]
            .iter()
            .map(|n| format!("test {n} ... ok\n"))
            .collect();
        let r = read(&mut e, &log);
        assert_eq!(r.patterns.len(), 1, "{:?}", r.patterns);
        let s = r.patterns.values().next().unwrap();
        assert_eq!(s.count, 6);
        assert_eq!(s.template, "test <*> ... ok");
        assert_eq!(s.line, 1);
    }
}
