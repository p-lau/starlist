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
    })
}

const PATTERN_COLS: &str = "p.id, p.template, p.level, p.exc_type, p.first_seen, p.last_seen, p.total";

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
             CREATE TABLE IF NOT EXISTS aliases(alias TEXT PRIMARY KEY, canonical TEXT NOT NULL) WITHOUT ROWID;",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record a batch of events in one transaction. Each event must have `ts` set.
    pub fn record(&self, events: &[Event]) -> R<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
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
            let mut set_sample = tx.prepare_cached("UPDATE patterns SET last_sample_ts = ?2 WHERE id = ?1")?;
            let mut prune = tx.prepare_cached(
                "DELETE FROM samples WHERE pattern_id = ?1 AND id NOT IN
                   (SELECT id FROM samples WHERE pattern_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2)",
            )?;
            for e in events {
                let ts = e.ts.expect("event timestamp must be set before recording");
                up_pattern.execute(params![e.pattern_id, e.template, e.level, e.exc_type, ts])?;
                up_count.execute(params![e.pattern_id, bucket_of(ts)])?;
                let last: Option<f64> = last_sample.query_row([&e.pattern_id], |r| r.get(0))?;
                if last.is_none_or(|l| (ts - l).abs() >= SAMPLE_EVERY_SECS) {
                    ins_sample.execute(params![e.pattern_id, ts, e.body])?;
                    set_sample.execute(params![e.pattern_id, ts])?;
                    prune.execute(params![e.pattern_id, SAMPLES_PER_PATTERN])?;
                }
            }
        }
        tx.commit()
    }

    /// Drop counts and markers older than `before_ts`. Patterns are kept so that
    /// "first seen" stays accurate.
    pub fn prune(&self, before_ts: f64) -> R<usize> {
        let conn = self.conn();
        conn.execute("DELETE FROM counts WHERE bucket < ?1", [bucket_of(before_ts)])
    }

    /// Look up a pattern by ID. IDs of patterns that were merged into another
    /// resolve to the pattern they were merged into.
    pub fn pattern(&self, id: &str) -> R<Option<PatternRow>> {
        let conn = self.conn();
        let id: String = conn
            .query_row("SELECT canonical FROM aliases WHERE alias = ?1", [id], |r| r.get(0))
            .optional()?
            .unwrap_or_else(|| id.to_string());
        conn.query_row(&format!("SELECT {PATTERN_COLS} FROM patterns p WHERE p.id = ?1"), [&id], |r| {
            row_to_pattern(r, 0)
        })
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
        tx.execute("UPDATE patterns SET template = ?2 WHERE id = ?1", params![c, op.template])?;
        for a in &op.aliases {
            tx.execute(
                "INSERT INTO counts(pattern_id, bucket, n) SELECT ?1, bucket, n FROM counts WHERE pattern_id = ?2 AND true
                 ON CONFLICT(pattern_id, bucket) DO UPDATE SET n = n + excluded.n",
                params![c, a],
            )?;
            tx.execute("DELETE FROM counts WHERE pattern_id = ?1", [a])?;
            tx.execute("UPDATE samples SET pattern_id = ?1 WHERE pattern_id = ?2", params![c, a])?;
            tx.execute(
                "UPDATE patterns SET
                   first_seen = MIN(first_seen, COALESCE((SELECT first_seen FROM patterns WHERE id = ?2), first_seen)),
                   last_seen = MAX(last_seen, COALESCE((SELECT last_seen FROM patterns WHERE id = ?2), last_seen)),
                   total = total + COALESCE((SELECT total FROM patterns WHERE id = ?2), 0)
                 WHERE id = ?1",
                params![c, a],
            )?;
            tx.execute("DELETE FROM patterns WHERE id = ?1", [a])?;
            tx.execute("UPDATE aliases SET canonical = ?1 WHERE canonical = ?2", params![c, a])?;
            tx.execute("INSERT OR REPLACE INTO aliases(alias, canonical) VALUES (?1, ?2)", params![a, c])?;
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
            Ok(SplitStat { row: row_to_pattern(r, 0)?, before: r.get(7)?, after: r.get(8)? })
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

    /// Adds the marker only if none with this name exists (used for deploy
    /// markers detected in logs, so replica restarts don't move the marker).
    pub fn add_marker_if_absent(&self, name: &str, ts: f64, kind: &str) -> R<bool> {
        if self.marker(name)?.is_some() {
            return Ok(false);
        }
        self.add_marker(name, ts, kind, None)?;
        Ok(true)
    }

    /// The most recent marker with this name.
    pub fn marker(&self, name: &str) -> R<Option<Marker>> {
        self.conn()
            .query_row(
                "SELECT name, ts, kind, note FROM markers WHERE name = ?1 ORDER BY ts DESC LIMIT 1",
                [name],
                |r| Ok(Marker { name: r.get(0)?, ts: r.get(1)?, kind: r.get(2)?, note: r.get(3)? }),
            )
            .optional()
    }

    /// Markers in [lo, hi], newest first.
    pub fn markers(&self, lo: f64, hi: f64, limit: i64) -> R<Vec<Marker>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT name, ts, kind, note FROM markers WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![lo, hi, limit], |r| {
            Ok(Marker { name: r.get(0)?, ts: r.get(1)?, kind: r.get(2)?, note: r.get(3)? })
        })?;
        rows.collect()
    }
}
