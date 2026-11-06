//! Local SQLite store: patterns, per-10-second counts, a few recent samples per
//! pattern, and markers (deploys, fixes). WAL mode lets a separate `mcp` process
//! read while the collector writes.

use crate::pattern::Event;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub const BUCKET_SECS: f64 = 10.0;
const SAMPLES_PER_PATTERN: i64 = 5;
const SAMPLE_EVERY_SECS: f64 = 10.0;

pub fn bucket_of(ts: f64) -> i64 {
    (ts / BUCKET_SECS).floor() as i64
}

#[derive(Debug, Clone)]
pub struct PatternRow {
    pub id: String,
    pub template: String,
    pub level: Option<String>,
    pub exc_type: Option<String>,
    pub first_seen: f64,
    pub last_seen: f64,
    pub total: i64,
    /// The most frequent code location of this pattern's events.
    pub location: Option<String>,
    /// The latest request (still in the raw buffer) that logged this pattern.
    pub example_request: Option<String>,
}

/// A pattern with its counts before and after a split time.
#[derive(Debug, Clone)]
pub struct SplitStat {
    pub row: PatternRow,
    pub before: i64,
    pub after: i64,
}

#[derive(Debug, Clone)]
pub struct Marker {
    pub name: String,
    pub ts: f64,
    pub kind: String,
    pub note: Option<String>,
}

pub struct Store {
    conn: Mutex<Connection>,
    raw: Mutex<RawBuffer>,
}

/// Default size of the raw-line buffer: the text of recent events, kept so an
/// agent can read the lines around an error.
pub const DEFAULT_RAW_BYTES: i64 = 20 * 1024 * 1024;
/// Longest single event kept in the raw buffer.
const RAW_EVENT_CHARS: usize = 4000;

struct RawBuffer {
    /// Upper bound on stored text in bytes; `None` keeps no raw lines.
    cap: Option<i64>,
    /// Bytes stored, counted on first write.
    bytes: Option<i64>,
}

/// One stored event in the raw buffer.
#[derive(Debug, Clone)]
pub struct RawLine {
    pub id: i64,
    pub ts: f64,
    pub pattern_id: String,
    pub text: String,
}

type R<T> = rusqlite::Result<T>;

fn row_to_pattern(r: &rusqlite::Row, off: usize) -> R<PatternRow> {
    Ok(PatternRow {
        id: r.get(off)?,
        template: r.get(off + 1)?,
        level: r.get(off + 2)?,
        exc_type: r.get(off + 3)?,
        first_seen: r.get(off + 4)?,
        last_seen: r.get(off + 5)?,
        total: r.get(off + 6)?,
        location: r.get(off + 7)?,
        example_request: r.get(off + 8)?,
    })
}

const PATTERN_COLS: &str = "p.id, p.template, p.level, p.exc_type, p.first_seen, p.last_seen, p.total,
     (SELECT l.location FROM locations l WHERE l.pattern_id = p.id ORDER BY l.n DESC, l.last_seen DESC LIMIT 1),
     (SELECT r.request_id FROM raw r WHERE r.pattern_id = p.id AND r.request_id IS NOT NULL ORDER BY r.ts DESC LIMIT 1)";

impl Store {
    pub fn open(path: impl AsRef<Path>) -> R<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS patterns(
                id TEXT PRIMARY KEY, template TEXT NOT NULL, level TEXT, exc_type TEXT,
                first_seen REAL NOT NULL, last_seen REAL NOT NULL, total INTEGER NOT NULL DEFAULT 0,
                last_sample_ts REAL);
             CREATE TABLE IF NOT EXISTS counts(
                pattern_id TEXT NOT NULL, bucket INTEGER NOT NULL, n INTEGER NOT NULL,
                PRIMARY KEY(pattern_id, bucket)) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS counts_bucket ON counts(bucket);
             CREATE TABLE IF NOT EXISTS samples(
                id INTEGER PRIMARY KEY, pattern_id TEXT NOT NULL, ts REAL NOT NULL, text TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS samples_pid ON samples(pattern_id, ts);
             CREATE TABLE IF NOT EXISTS markers(
                id INTEGER PRIMARY KEY, name TEXT NOT NULL, ts REAL NOT NULL, kind TEXT NOT NULL, note TEXT);
             CREATE INDEX IF NOT EXISTS markers_name ON markers(name);
             CREATE TABLE IF NOT EXISTS aliases(alias TEXT PRIMARY KEY, canonical TEXT NOT NULL) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS locations(
                pattern_id TEXT NOT NULL, location TEXT NOT NULL, n INTEGER NOT NULL, last_seen REAL NOT NULL,
                PRIMARY KEY(pattern_id, location)) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS raw(
                id INTEGER PRIMARY KEY, ts REAL NOT NULL, pattern_id TEXT NOT NULL, text TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS raw_ts ON raw(ts, id);
             CREATE INDEX IF NOT EXISTS raw_pid ON raw(pattern_id, ts);",
        )?;
        // Added in 0.6: the request ID of each raw line.
        let has_req: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('raw') WHERE name = 'request_id'")?
            .exists([])?;
        if !has_req {
            conn.execute_batch("ALTER TABLE raw ADD COLUMN request_id TEXT;")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS raw_req ON raw(request_id, ts) WHERE request_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS raw_pid_req ON raw(pattern_id, ts) WHERE request_id IS NOT NULL;",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            raw: Mutex::new(RawBuffer {
                cap: Some(DEFAULT_RAW_BYTES),
                bytes: None,
            }),
        })
    }

    /// Set the raw-line buffer size in bytes, or turn it off with `None`,
    /// which also deletes the lines already stored.
    pub fn set_raw_cap(&self, cap: Option<i64>) -> R<()> {
        let mut raw = self.raw.lock().unwrap_or_else(|e| e.into_inner());
        raw.cap = cap.map(|c| c.max(0));
        if raw.cap.is_none() {
            self.conn().execute("DELETE FROM raw", [])?;
            raw.bytes = Some(0);
        }
        Ok(())
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record a batch of events in one transaction. Each event must have `ts` set.
    pub fn record(&self, events: &[Event]) -> R<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut raw = self.raw.lock().unwrap_or_else(|e| e.into_inner());
        let mut conn = self.conn();
        if raw.cap.is_some() && raw.bytes.is_none() {
            raw.bytes = Some(conn.query_row(
                "SELECT COALESCE(SUM(length(CAST(text AS BLOB))), 0) FROM raw",
                [],
                |r| r.get(0),
            )?);
        }
        let tx = conn.transaction()?;
        {
            let mut ins_raw = tx.prepare_cached(
                "INSERT INTO raw(ts, pattern_id, text, request_id) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut up_pattern = tx.prepare_cached(
                "INSERT INTO patterns(id, template, level, exc_type, first_seen, last_seen, total)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, 1)
                 ON CONFLICT(id) DO UPDATE SET
                   first_seen = MIN(first_seen, excluded.first_seen),
                   last_seen = MAX(last_seen, excluded.last_seen),
                   total = total + 1,
                   level = COALESCE(level, excluded.level),
                   exc_type = COALESCE(exc_type, excluded.exc_type)",
            )?;
            let mut up_count = tx.prepare_cached(
                "INSERT INTO counts(pattern_id, bucket, n) VALUES (?1, ?2, 1)
                 ON CONFLICT(pattern_id, bucket) DO UPDATE SET n = n + 1",
            )?;
            let mut last_sample =
                tx.prepare_cached("SELECT last_sample_ts FROM patterns WHERE id = ?1")?;
            let mut ins_sample =
                tx.prepare_cached("INSERT INTO samples(pattern_id, ts, text) VALUES (?1, ?2, ?3)")?;
            let mut set_sample =
                tx.prepare_cached("UPDATE patterns SET last_sample_ts = ?2 WHERE id = ?1")?;
            let mut up_loc = tx.prepare_cached(
                "INSERT INTO locations(pattern_id, location, n, last_seen) VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(pattern_id, location) DO UPDATE SET
                   n = n + 1, last_seen = MAX(last_seen, excluded.last_seen)",
            )?;
            let mut prune = tx.prepare_cached(
                "DELETE FROM samples WHERE pattern_id = ?1 AND id NOT IN
                   (SELECT id FROM samples WHERE pattern_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2)",
            )?;
            for e in events {
                let ts = e.ts.expect("event timestamp must be set before recording");
                up_pattern.execute(params![e.pattern_id, e.template, e.level, e.exc_type, ts])?;
                up_count.execute(params![e.pattern_id, bucket_of(ts)])?;
                if let Some(l) = &e.location {
                    up_loc.execute(params![e.pattern_id, l, ts])?;
                }
                if raw.cap.is_some() {
                    let text = crate::pattern::truncate(&e.body, RAW_EVENT_CHARS);
                    ins_raw.execute(params![ts, e.pattern_id, text, e.request_id])?;
                    *raw.bytes.get_or_insert(0) += text.len() as i64;
                }
                let last: Option<f64> = last_sample.query_row([&e.pattern_id], |r| r.get(0))?;
                if last.is_none_or(|l| (ts - l).abs() >= SAMPLE_EVERY_SECS) {
                    ins_sample.execute(params![e.pattern_id, ts, e.body])?;
                    set_sample.execute(params![e.pattern_id, ts])?;
                    prune.execute(params![e.pattern_id, SAMPLES_PER_PATTERN])?;
                }
            }
            // Over the cap: drop the oldest lines until 90% of it is used.
            if let (Some(cap), Some(bytes)) = (raw.cap, raw.bytes)
                && bytes > cap
            {
                let target = bytes - cap * 9 / 10;
                let mut st = tx
                    .prepare_cached("SELECT id, length(CAST(text AS BLOB)) FROM raw ORDER BY id")?;
                let mut rows = st.query([])?;
                let (mut freed, mut cut) = (0i64, None);
                while freed < target
                    && let Some(r) = rows.next()?
                {
                    cut = Some(r.get::<_, i64>(0)?);
                    freed += r.get::<_, i64>(1)?;
                }
                drop(rows);
                if let Some(cut) = cut {
                    tx.execute("DELETE FROM raw WHERE id <= ?1", [cut])?;
                    raw.bytes = Some(bytes - freed);
                }
            }
        }
        tx.commit()
    }

    /// The raw line to center a context window on: the latest occurrence of
    /// `pattern_id` at or before `at` (or ever), or the last line at or before `at`.
    pub fn raw_anchor(&self, pattern_id: Option<&str>, at: Option<f64>) -> R<Option<RawLine>> {
        let at = at.unwrap_or(f64::MAX);
        let conn = self.conn();
        let row = |r: &rusqlite::Row| {
            Ok(RawLine {
                id: r.get(0)?,
                ts: r.get(1)?,
                pattern_id: r.get(2)?,
                text: r.get(3)?,
            })
        };
        match pattern_id {
            Some(p) => conn
                .query_row(
                    "SELECT id, ts, pattern_id, text FROM raw WHERE pattern_id = ?1 AND ts <= ?2
                     ORDER BY ts DESC, id DESC LIMIT 1",
                    params![p, at],
                    row,
                )
                .optional(),
            None => conn
                .query_row(
                    "SELECT id, ts, pattern_id, text FROM raw WHERE ts <= ?1
                     ORDER BY ts DESC, id DESC LIMIT 1",
                    params![at],
                    row,
                )
                .optional(),
        }
    }

    /// Up to `before` lines before and `after` lines after the anchor, oldest first.
    pub fn raw_around(&self, anchor: &RawLine, before: i64, after: i64) -> R<Vec<RawLine>> {
        let conn = self.conn();
        let row = |r: &rusqlite::Row| {
            Ok(RawLine {
                id: r.get(0)?,
                ts: r.get(1)?,
                pattern_id: r.get(2)?,
                text: r.get(3)?,
            })
        };
        let mut st = conn.prepare_cached(
            "SELECT id, ts, pattern_id, text FROM raw WHERE (ts, id) < (?1, ?2)
             ORDER BY ts DESC, id DESC LIMIT ?3",
        )?;
        let mut lines: Vec<RawLine> = st
            .query_map(params![anchor.ts, anchor.id, before], row)?
            .collect::<R<_>>()?;
        lines.reverse();
        lines.push(anchor.clone());
        let mut st = conn.prepare_cached(
            "SELECT id, ts, pattern_id, text FROM raw WHERE (ts, id) > (?1, ?2)
             ORDER BY ts, id LIMIT ?3",
        )?;
        for l in st.query_map(params![anchor.ts, anchor.id, after], row)? {
            lines.push(l?);
        }
        Ok(lines)
    }

    /// The events of one request in order, at most `limit`.
    pub fn request_lines(&self, request_id: &str, limit: i64) -> R<Vec<RawLine>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, ts, pattern_id, text FROM raw WHERE request_id = ?1 ORDER BY ts, id LIMIT ?2",
        )?;
        let rows = st.query_map(params![request_id, limit], |r| {
            Ok(RawLine {
                id: r.get(0)?,
                ts: r.get(1)?,
                pattern_id: r.get(2)?,
                text: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Number of raw lines stored and the oldest one's time.
    pub fn raw_span(&self) -> R<(i64, Option<f64>)> {
        self.conn()
            .query_row("SELECT COUNT(*), MIN(ts) FROM raw", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
    }

    /// Drop counts and markers older than `before_ts`. Patterns are kept so that
    /// "first seen" stays accurate.
    pub fn prune(&self, before_ts: f64) -> R<usize> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM counts WHERE bucket < ?1",
            [bucket_of(before_ts)],
        )
    }

    /// Look up a pattern by ID. IDs of patterns that were merged into another
    /// resolve to the pattern they were merged into.
    pub fn pattern(&self, id: &str) -> R<Option<PatternRow>> {
        let conn = self.conn();
        let id: String = conn
            .query_row(
                "SELECT canonical FROM aliases WHERE alias = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| id.to_string());
        conn.query_row(
            &format!("SELECT {PATTERN_COLS} FROM patterns p WHERE p.id = ?1"),
            [&id],
            |r| row_to_pattern(r, 0),
        )
        .optional()
    }

    pub fn aliases(&self) -> R<std::collections::HashMap<String, String>> {
        let conn = self.conn();
        let mut st = conn.prepare("SELECT alias, canonical FROM aliases")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// All patterns' IDs and templates, oldest first.
    pub fn pattern_templates(&self) -> R<Vec<(String, String)>> {
        let conn = self.conn();
        let mut st = conn.prepare("SELECT id, template FROM patterns ORDER BY first_seen, id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Fold the alias patterns into the canonical one: counts, samples and
    /// totals move over, and the alias IDs keep resolving to the canonical ID.
    pub fn merge_patterns(&self, op: &crate::merge::MergeOp) -> R<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let c = &op.canonical;
        tx.execute(
            "UPDATE patterns SET template = ?2 WHERE id = ?1",
            params![c, op.template],
        )?;
        for a in &op.aliases {
            tx.execute(
                "INSERT INTO counts(pattern_id, bucket, n) SELECT ?1, bucket, n FROM counts WHERE pattern_id = ?2 AND true
                 ON CONFLICT(pattern_id, bucket) DO UPDATE SET n = n + excluded.n",
                params![c, a],
            )?;
            tx.execute("DELETE FROM counts WHERE pattern_id = ?1", [a])?;
            tx.execute(
                "INSERT INTO locations(pattern_id, location, n, last_seen)
                 SELECT ?1, location, n, last_seen FROM locations WHERE pattern_id = ?2 AND true
                 ON CONFLICT(pattern_id, location) DO UPDATE SET
                   n = n + excluded.n, last_seen = MAX(last_seen, excluded.last_seen)",
                params![c, a],
            )?;
            tx.execute("DELETE FROM locations WHERE pattern_id = ?1", [a])?;
            tx.execute(
                "UPDATE raw SET pattern_id = ?1 WHERE pattern_id = ?2",
                params![c, a],
            )?;
            tx.execute(
                "UPDATE samples SET pattern_id = ?1 WHERE pattern_id = ?2",
                params![c, a],
            )?;
            tx.execute(
                "UPDATE patterns SET
                   first_seen = MIN(first_seen, COALESCE((SELECT first_seen FROM patterns WHERE id = ?2), first_seen)),
                   last_seen = MAX(last_seen, COALESCE((SELECT last_seen FROM patterns WHERE id = ?2), last_seen)),
                   total = total + COALESCE((SELECT total FROM patterns WHERE id = ?2), 0)
                 WHERE id = ?1",
                params![c, a],
            )?;
            tx.execute("DELETE FROM patterns WHERE id = ?1", [a])?;
            tx.execute(
                "UPDATE aliases SET canonical = ?1 WHERE canonical = ?2",
                params![c, a],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO aliases(alias, canonical) VALUES (?1, ?2)",
                params![a, c],
            )?;
        }
        tx.execute(
            "DELETE FROM samples WHERE pattern_id = ?1 AND id NOT IN
               (SELECT id FROM samples WHERE pattern_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2)",
            params![c, SAMPLES_PER_PATTERN],
        )?;
        tx.commit()
    }

    /// Per-pattern counts in [lo, split) and [split, hi]. The 10-second bucket
    /// that contains `split` is left out of both sides, so events right around
    /// a deploy are not misattributed.
    pub fn split_counts(&self, lo: f64, split: f64, hi: f64) -> R<Vec<SplitStat>> {
        let (lo_b, split_b, hi_b) = (bucket_of(lo), bucket_of(split), bucket_of(hi));
        let conn = self.conn();
        let mut st = conn.prepare_cached(&format!(
            "SELECT {PATTERN_COLS},
                    SUM(CASE WHEN c.bucket < ?2 THEN c.n ELSE 0 END),
                    SUM(CASE WHEN c.bucket > ?2 THEN c.n ELSE 0 END)
             FROM counts c JOIN patterns p ON p.id = c.pattern_id
             WHERE c.bucket >= ?1 AND c.bucket <= ?3
             GROUP BY c.pattern_id"
        ))?;
        let rows = st.query_map(params![lo_b, split_b, hi_b], |r| {
            Ok(SplitStat {
                row: row_to_pattern(r, 0)?,
                before: r.get(9)?,
                after: r.get(10)?,
            })
        })?;
        rows.collect()
    }

    pub fn count(&self, pattern_id: &str, lo: f64, hi: f64) -> R<i64> {
        self.conn().query_row(
            "SELECT COALESCE(SUM(n), 0) FROM counts WHERE pattern_id = ?1 AND bucket >= ?2 AND bucket <= ?3",
            params![pattern_id, bucket_of(lo), bucket_of(hi)],
            |r| r.get(0),
        )
    }

    /// Total events of all patterns, before and after the split (same bucket rules as split_counts).
    pub fn total_split(&self, lo: f64, split: f64, hi: f64) -> R<(i64, i64)> {
        self.conn().query_row(
            "SELECT COALESCE(SUM(CASE WHEN bucket < ?2 THEN n ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN bucket > ?2 THEN n ELSE 0 END), 0)
             FROM counts WHERE bucket >= ?1 AND bucket <= ?3",
            params![bucket_of(lo), bucket_of(split), bucket_of(hi)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// Counts per `step` seconds from `lo` up to `hi`, oldest first.
    pub fn histogram(&self, pattern_id: &str, lo: f64, hi: f64, step: f64) -> R<Vec<i64>> {
        let slots = ((hi - lo) / step).ceil().max(1.0) as usize;
        let mut out = vec![0i64; slots];
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT bucket, n FROM counts WHERE pattern_id = ?1 AND bucket >= ?2 AND bucket <= ?3",
        )?;
        let rows = st.query_map(params![pattern_id, bucket_of(lo), bucket_of(hi)], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (b, n) = row?;
            let t = b as f64 * BUCKET_SECS;
            let i = (((t - lo) / step).floor().max(0.0) as usize).min(slots - 1);
            out[i] += n;
        }
        Ok(out)
    }

    /// Event counts per level per `step` seconds from `lo` up to `hi`, oldest
    /// first. Lines without a level are reported as INFO, as elsewhere.
    pub fn level_series(&self, lo: f64, hi: f64, step: f64) -> R<Vec<(String, Vec<i64>)>> {
        let slots = ((hi - lo) / step).ceil().max(1.0) as usize;
        let mut out: Vec<(String, Vec<i64>)> = vec![];
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT COALESCE(p.level, 'INFO'), c.bucket, SUM(c.n)
             FROM counts c JOIN patterns p ON p.id = c.pattern_id
             WHERE c.bucket >= ?1 AND c.bucket <= ?2
             GROUP BY 1, c.bucket",
        )?;
        let rows = st.query_map(params![bucket_of(lo), bucket_of(hi)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (level, b, n) = row?;
            let t = b as f64 * BUCKET_SECS;
            let i = (((t - lo) / step).floor().max(0.0) as usize).min(slots - 1);
            let k = match out.iter().position(|(l, _)| *l == level) {
                Some(k) => k,
                None => {
                    out.push((level, vec![0; slots]));
                    out.len() - 1
                }
            };
            out[k].1[i] += n;
        }
        Ok(out)
    }

    /// Number of patterns (merged ones count once).
    pub fn pattern_count(&self) -> R<i64> {
        self.conn()
            .query_row("SELECT COUNT(*) FROM patterns", [], |r| r.get(0))
    }

    /// Code locations of a pattern's events, most frequent first.
    pub fn locations(&self, pattern_id: &str, limit: i64) -> R<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT location, n FROM locations WHERE pattern_id = ?1 ORDER BY n DESC, last_seen DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![pattern_id, limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn samples(&self, pattern_id: &str, limit: i64) -> R<Vec<(f64, String)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT ts, text FROM samples WHERE pattern_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![pattern_id, limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn add_marker(&self, name: &str, ts: f64, kind: &str, note: Option<&str>) -> R<()> {
        self.conn().execute(
            "INSERT INTO markers(name, ts, kind, note) VALUES (?1, ?2, ?3, ?4)",
            params![name, ts, kind, note],
        )?;
        Ok(())
    }

    /// Record that release `name` went live at `ts`, unless it was already
    /// the live release then. Replica restarts and re-imported logs don't add
    /// duplicates, while a rollback to an older release is still recorded.
    pub fn add_deploy(&self, name: &str, ts: f64, note: Option<&str>) -> R<bool> {
        let live: Option<String> = self
            .conn()
            .query_row(
                "SELECT name FROM markers WHERE kind = 'deploy' AND ts <= ?1 ORDER BY ts DESC, id DESC LIMIT 1",
                [ts],
                |r| r.get(0),
            )
            .optional()?;
        if live.as_deref() == Some(name) {
            return Ok(false);
        }
        self.add_marker(name, ts, "deploy", note)?;
        Ok(true)
    }

    /// The most recent marker with this name.
    pub fn marker(&self, name: &str) -> R<Option<Marker>> {
        self.conn()
            .query_row(
                "SELECT name, ts, kind, note FROM markers WHERE name = ?1 ORDER BY ts DESC LIMIT 1",
                [name],
                |r| {
                    Ok(Marker {
                        name: r.get(0)?,
                        ts: r.get(1)?,
                        kind: r.get(2)?,
                        note: r.get(3)?,
                    })
                },
            )
            .optional()
    }

    /// Deploy markers (name, time), oldest first.
    pub fn deploys(&self) -> R<Vec<(f64, String)>> {
        let conn = self.conn();
        let mut st = conn
            .prepare_cached("SELECT ts, name FROM markers WHERE kind = 'deploy' ORDER BY ts, id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Markers in [lo, hi], newest first.
    pub fn markers(&self, lo: f64, hi: f64, limit: i64) -> R<Vec<Marker>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT name, ts, kind, note FROM markers WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![lo, hi, limit], |r| {
            Ok(Marker {
                name: r.get(0)?,
                ts: r.get(1)?,
                kind: r.get(2)?,
                note: r.get(3)?,
            })
        })?;
        rows.collect()
    }
}
