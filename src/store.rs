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

/// A pattern's issue state as stored. See `issues` for what it means now.
#[derive(Debug, Clone, PartialEq)]
pub struct StateRow {
    pub pattern_id: String,
    /// "resolved", "resolved_next" or "ignored".
    pub state: String,
    /// When the state was set.
    pub since: f64,
    /// Ignored until this time.
    pub until_ts: Option<f64>,
    /// Ignored until this many more events.
    pub until_count: Option<i64>,
    /// The pattern's total when the state was set.
    pub base_total: i64,
    pub note: Option<String>,
}

pub struct Store {
    conn: Mutex<Connection>,
    raw: Mutex<RawBuffer>,
    /// Set when pushing to a hub: what gets recorded is also queued to send.
    outbox: Mutex<Option<Outbox>>,
}

struct Outbox {
    /// Most bytes of queued JSON; the oldest entries are dropped beyond it.
    cap: i64,
    /// Bytes queued, counted on first write.
    bytes: Option<i64>,
    /// Send events without raw text for the hub's buffer.
    no_raw: bool,
}

/// Queue an item for the hub in the current transaction.
fn queue(tx: &rusqlite::Connection, ob: &mut Outbox, item: &crate::push::Item) -> R<()> {
    let body = serde_json::to_string(item).unwrap_or_default();
    if ob.bytes.is_none() {
        ob.bytes = Some(tx.query_row(
            "SELECT COALESCE(SUM(length(CAST(body AS BLOB))), 0) FROM outbox",
            [],
            |r| r.get(0),
        )?);
    }
    tx.prepare_cached("INSERT INTO outbox(body) VALUES (?1)")?
        .execute([&body])?;
    let bytes = ob.bytes.get_or_insert(0);
    *bytes += body.len() as i64;
    if *bytes > ob.cap {
        // Drop the oldest entries until 90% of the cap is used.
        let target = *bytes - ob.cap * 9 / 10;
        let mut st =
            tx.prepare_cached("SELECT seq, length(CAST(body AS BLOB)) FROM outbox ORDER BY seq")?;
        let mut rows = st.query([])?;
        let (mut freed, mut upto, mut n) = (0i64, 0i64, 0i64);
        while freed < target
            && let Some(r) = rows.next()?
        {
            upto = r.get(0)?;
            freed += r.get::<_, i64>(1)?;
            n += 1;
        }
        drop(rows);
        tx.execute("DELETE FROM outbox WHERE seq <= ?1", [upto])?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('hub_dropped', ?1)
             ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + ?1",
            [n],
        )?;
        *bytes -= freed;
    }
    Ok(())
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
        // Added in 0.10: issue states and collector settings.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pattern_state(
                pattern_id TEXT PRIMARY KEY, state TEXT NOT NULL, since REAL NOT NULL,
                until_ts REAL, until_count INTEGER, base_total INTEGER NOT NULL DEFAULT 0,
                note TEXT) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;",
        )?;
        // Added in 0.11: hourly rollups of old counts, and numeric fields.
        conn.execute_batch(
            "-- `bucket` is the hour's first 10-second bucket, so it reads like `counts`.
             CREATE TABLE IF NOT EXISTS hourly(
                pattern_id TEXT NOT NULL, bucket INTEGER NOT NULL, n INTEGER NOT NULL,
                PRIMARY KEY(pattern_id, bucket)) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS hourly_bucket ON hourly(bucket);
             CREATE TABLE IF NOT EXISTS metric_hourly(
                pattern_id TEXT NOT NULL, field TEXT NOT NULL, hour INTEGER NOT NULL,
                n INTEGER NOT NULL, sum REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL,
                bins TEXT NOT NULL,
                PRIMARY KEY(pattern_id, field, hour)) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS metric_hour ON metric_hourly(field, hour);
             -- What a collector still has to push to its hub.
             CREATE TABLE IF NOT EXISTS outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL);
             -- Recent 10-second counts and older hourly ones, read as one table.
             CREATE TEMP VIEW IF NOT EXISTS allc AS
                SELECT pattern_id, bucket, n FROM counts
                UNION ALL SELECT pattern_id, bucket, n FROM hourly;",
        )?;
        // Per-pattern timing overrides: "usual" or "always_alert".
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pattern_timing(pattern_id TEXT PRIMARY KEY, mode TEXT NOT NULL) WITHOUT ROWID;",
        )?;
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
            outbox: Mutex::new(None),
        })
    }

    /// Queue everything recorded from now on for a hub, keeping at most
    /// `cap` bytes queued. `no_raw` sends events without raw text for the
    /// hub's buffer.
    pub fn enable_outbox(&self, cap: i64, no_raw: bool) {
        *self.outbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(Outbox {
            cap: cap.max(1 << 20),
            bytes: None,
            no_raw,
        });
    }

    /// The oldest queued items, up to about `max_bytes`: (seq, JSON).
    pub fn outbox_batch(&self, max_bytes: usize) -> R<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT seq, body FROM outbox ORDER BY seq LIMIT 2000")?;
        let mut rows = st.query([])?;
        let (mut out, mut size) = (vec![], 0usize);
        while let Some(r) = rows.next()? {
            let body: String = r.get(1)?;
            if !out.is_empty() && size + body.len() > max_bytes {
                break;
            }
            size += body.len();
            out.push((r.get(0)?, body));
        }
        Ok(out)
    }

    /// The hub applied everything up to `seq`.
    pub fn outbox_ack(&self, seq: i64) -> R<()> {
        let conn = self.conn();
        let freed: i64 = conn.query_row(
            "SELECT COALESCE(SUM(length(CAST(body AS BLOB))), 0) FROM outbox WHERE seq <= ?1",
            [seq],
            |r| r.get(0),
        )?;
        conn.execute("DELETE FROM outbox WHERE seq <= ?1", [seq])?;
        if let Some(ob) = self
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            && let Some(b) = ob.bytes.as_mut()
        {
            *b = (*b - freed).max(0);
        }
        Ok(())
    }

    /// Items waiting to be pushed, and their size in bytes.
    pub fn outbox_len(&self) -> R<(i64, i64)> {
        self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(CAST(body AS BLOB))), 0) FROM outbox",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
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
}

/// A histogram from columns n, sum, min, max, bins starting at `i`.
fn hist_at(r: &rusqlite::Row, i: usize) -> R<crate::metrics::Hist> {
    Ok(crate::metrics::Hist::from_parts(
        r.get::<_, i64>(i)? as u64,
        r.get(i + 1)?,
        r.get(i + 2)?,
        r.get(i + 3)?,
        &r.get::<_, String>(i + 4)?,
    ))
}

/// Add a histogram to the stored one for (pattern, field, hour).
fn add_hist(
    tx: &rusqlite::Transaction,
    pid: &str,
    field: &str,
    hour: i64,
    mut h: crate::metrics::Hist,
) -> R<()> {
    let old = tx
        .prepare_cached(
            "SELECT n, sum, min, max, bins FROM metric_hourly WHERE pattern_id = ?1 AND field = ?2 AND hour = ?3",
        )?
        .query_row(params![pid, field, hour], |r| hist_at(r, 0))
        .optional()?;
    if let Some(o) = old {
        h.merge(&o);
    }
    tx.prepare_cached(
        "INSERT OR REPLACE INTO metric_hourly(pattern_id, field, hour, n, sum, min, max, bins)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![
        pid,
        field,
        hour,
        h.n as i64,
        h.sum,
        h.min,
        h.max,
        h.bins_text()
    ])?;
    Ok(())
}

impl Store {
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
            // Patterns merged by hand (or by another process) keep resolving
            // to the pattern they were merged into.
            let aliases: std::collections::HashMap<String, String> = tx
                .prepare_cached("SELECT alias, canonical FROM aliases")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<R<_>>()?;
            for e in events {
                let ts = e.ts.expect("event timestamp must be set before recording");
                let pid = aliases.get(&e.pattern_id).unwrap_or(&e.pattern_id);
                up_pattern.execute(params![pid, e.template, e.level, e.exc_type, ts])?;
                up_count.execute(params![pid, bucket_of(ts)])?;
                if let Some(l) = &e.location {
                    up_loc.execute(params![pid, l, ts])?;
                }
                if raw.cap.is_some() && !e.no_raw {
                    let text = crate::pattern::truncate(&e.body, RAW_EVENT_CHARS);
                    ins_raw.execute(params![ts, pid, text, e.request_id])?;
                    *raw.bytes.get_or_insert(0) += text.len() as i64;
                }
                let last: Option<f64> = last_sample.query_row([pid], |r| r.get(0))?;
                if last.is_none_or(|l| (ts - l).abs() >= SAMPLE_EVERY_SECS) {
                    ins_sample.execute(params![pid, ts, e.body])?;
                    set_sample.execute(params![pid, ts])?;
                    prune.execute(params![pid, SAMPLES_PER_PATTERN])?;
                }
            }
            // Numeric fields: one histogram per pattern, field and hour.
            let mut hists: std::collections::HashMap<(String, String, i64), crate::metrics::Hist> =
                Default::default();
            for e in events.iter().filter(|e| !e.metrics.is_empty()) {
                let ts = e.ts.unwrap_or_default();
                let pid = aliases.get(&e.pattern_id).unwrap_or(&e.pattern_id);
                for (f, v) in &e.metrics {
                    hists
                        .entry((pid.clone(), f.clone(), (ts / 3600.0).floor() as i64))
                        .or_default()
                        .add(*v);
                }
            }
            for ((pid, f, hour), h) in hists {
                add_hist(&tx, &pid, &f, hour, h)?;
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
        if let Some(ob) = self
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let no_raw = ob.no_raw || raw.cap.is_none();
            let item = crate::push::Item::Events {
                events: events
                    .iter()
                    .map(|e| crate::push::WireEvent::from_event(e, no_raw))
                    .collect(),
            };
            queue(&tx, ob, &item)?;
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

    /// Fold counts older than `before_ts` into hourly counts. Patterns are kept
    /// so that "first seen" stays accurate.
    pub fn prune(&self, before_ts: f64) -> R<usize> {
        self.rollup(before_ts, None)
    }

    /// Fold 10-second counts older than `before_ts` into hourly counts, then
    /// drop hourly counts and metrics older than `history_before`.
    /// Returns the number of 10-second rows folded.
    pub fn rollup(&self, before_ts: f64, history_before: Option<f64>) -> R<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let b = bucket_of(before_ts);
        tx.execute(
            "INSERT INTO hourly(pattern_id, bucket, n)
             SELECT pattern_id, bucket / 360 * 360, SUM(n) FROM counts WHERE bucket < ?1 GROUP BY 1, 2
             ON CONFLICT(pattern_id, bucket) DO UPDATE SET n = n + excluded.n",
            [b],
        )?;
        let folded = tx.execute("DELETE FROM counts WHERE bucket < ?1", [b])?;
        if let Some(h) = history_before {
            let hour = (h / 3600.0).floor() as i64;
            tx.execute("DELETE FROM hourly WHERE bucket < ?1", [hour * 360])?;
            tx.execute("DELETE FROM metric_hourly WHERE hour < ?1", [hour])?;
        }
        tx.commit()?;
        Ok(folded)
    }

    /// Histogram of `field` per hour from `lo` to `hi`, for one pattern or all.
    pub fn metric_hours(
        &self,
        field: &str,
        pattern_id: Option<&str>,
        lo: f64,
        hi: f64,
    ) -> R<Vec<(i64, crate::metrics::Hist)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT hour, n, sum, min, max, bins FROM metric_hourly
             WHERE field = ?1 AND (?2 IS NULL OR pattern_id = ?2) AND hour >= ?3 AND hour <= ?4
             ORDER BY hour",
        )?;
        let rows = st.query_map(
            params![
                field,
                pattern_id,
                (lo / 3600.0).floor() as i64,
                (hi / 3600.0).floor() as i64
            ],
            |r| Ok((r.get::<_, i64>(0)?, hist_at(r, 1)?)),
        )?;
        // One entry per hour, patterns combined.
        let mut out: Vec<(i64, crate::metrics::Hist)> = vec![];
        for row in rows {
            let (h, hist) = row?;
            match out.last_mut() {
                Some((lh, acc)) if *lh == h => acc.merge(&hist),
                _ => out.push((h, hist)),
            }
        }
        Ok(out)
    }

    /// Fields recorded for a pattern (or any), most used first.
    pub fn metric_fields(&self, pattern_id: Option<&str>) -> R<Vec<String>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT field FROM metric_hourly WHERE ?1 IS NULL OR pattern_id = ?1
             GROUP BY field ORDER BY SUM(n) DESC",
        )?;
        let rows = st.query_map(params![pattern_id], |r| r.get(0))?;
        rows.collect()
    }

    /// Per-level totals since the database began, for counters.
    pub fn level_totals(&self) -> R<Vec<(String, i64, i64)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT COALESCE(level, 'INFO'), SUM(total), COUNT(*) FROM patterns GROUP BY 1",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }

    /// Database file size in bytes.
    pub fn size_bytes(&self) -> R<i64> {
        self.conn().query_row(
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
            [],
            |r| r.get(0),
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
    /// A pattern merger that continues from the stored patterns and aliases,
    /// applying merges they already qualify for.
    pub fn merger(&self, threshold: usize) -> R<crate::merge::Merger> {
        crate::merge::Merger::restore(
            threshold,
            self.aliases()?,
            self.pattern_templates()?,
            |op| self.merge_patterns(op),
        )
    }

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
                "INSERT INTO hourly(pattern_id, bucket, n) SELECT ?1, bucket, n FROM hourly WHERE pattern_id = ?2 AND true
                 ON CONFLICT(pattern_id, bucket) DO UPDATE SET n = n + excluded.n",
                params![c, a],
            )?;
            tx.execute("DELETE FROM hourly WHERE pattern_id = ?1", [a])?;
            let moved: Vec<(String, i64, crate::metrics::Hist)> = {
                let mut st = tx.prepare_cached(
                    "SELECT field, hour, n, sum, min, max, bins FROM metric_hourly WHERE pattern_id = ?1",
                )?;
                st.query_map([a], |r| Ok((r.get(0)?, r.get(1)?, hist_at(r, 2)?)))?
                    .collect::<R<_>>()?
            };
            for (f, hour, h) in moved {
                add_hist(&tx, c, &f, hour, h)?;
            }
            tx.execute("DELETE FROM metric_hourly WHERE pattern_id = ?1", [a])?;
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
            tx.execute("DELETE FROM pattern_state WHERE pattern_id = ?1", [a])?;
            tx.execute("DELETE FROM pattern_timing WHERE pattern_id = ?1", [a])?;
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
        if let Some(ob) = self
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let item = crate::push::Item::Merge {
                canonical: op.canonical.clone(),
                template: op.template.clone(),
                aliases: op.aliases.clone(),
            };
            queue(&tx, ob, &item)?;
        }
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
             FROM allc c JOIN patterns p ON p.id = c.pattern_id
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
            "SELECT COALESCE(SUM(n), 0) FROM allc WHERE pattern_id = ?1 AND bucket >= ?2 AND bucket <= ?3",
            params![pattern_id, bucket_of(lo), bucket_of(hi)],
            |r| r.get(0),
        )
    }

    /// Total events of all patterns, before and after the split (same bucket rules as split_counts).
    pub fn total_split(&self, lo: f64, split: f64, hi: f64) -> R<(i64, i64)> {
        self.conn().query_row(
            "SELECT COALESCE(SUM(CASE WHEN bucket < ?2 THEN n ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN bucket > ?2 THEN n ELSE 0 END), 0)
             FROM allc WHERE bucket >= ?1 AND bucket <= ?3",
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
            "SELECT bucket, n FROM allc WHERE pattern_id = ?1 AND bucket >= ?2 AND bucket <= ?3",
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
             FROM allc c JOIN patterns p ON p.id = c.pattern_id
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

    /// Every pattern's stored state (patterns without one are open).
    pub fn states(&self) -> R<std::collections::HashMap<String, StateRow>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT pattern_id, state, since, until_ts, until_count, base_total, note FROM pattern_state",
        )?;
        let rows = st.query_map([], |r| {
            Ok(StateRow {
                pattern_id: r.get(0)?,
                state: r.get(1)?,
                since: r.get(2)?,
                until_ts: r.get(3)?,
                until_count: r.get(4)?,
                base_total: r.get(5)?,
                note: r.get(6)?,
            })
        })?;
        rows.map(|r| r.map(|s| (s.pattern_id.clone(), s))).collect()
    }

    /// Set a pattern's state, or clear it (`None` = open).
    pub fn set_state(&self, pattern_id: &str, row: Option<&StateRow>) -> R<()> {
        let conn = self.conn();
        match row {
            None => conn.execute("DELETE FROM pattern_state WHERE pattern_id = ?1", [pattern_id])?,
            Some(s) => conn.execute(
                "INSERT OR REPLACE INTO pattern_state(pattern_id, state, since, until_ts, until_count, base_total, note)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![pattern_id, s.state, s.since, s.until_ts, s.until_count, s.base_total, s.note],
            )?,
        };
        Ok(())
    }

    /// Every pattern's timing override (patterns without one are judged from history).
    pub fn timings(&self) -> R<std::collections::HashMap<String, String>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT pattern_id, mode FROM pattern_timing")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Set a pattern's timing override, or clear it (`None`).
    pub fn set_timing(&self, pattern_id: &str, mode: Option<&str>) -> R<()> {
        let conn = self.conn();
        match mode {
            None => conn.execute(
                "DELETE FROM pattern_timing WHERE pattern_id = ?1",
                [pattern_id],
            )?,
            Some(m) => conn.execute(
                "INSERT OR REPLACE INTO pattern_timing(pattern_id, mode) VALUES (?1, ?2)",
                params![pattern_id, m],
            )?,
        };
        Ok(())
    }

    /// Start of the first 10-second bucket after `ts` in which the pattern occurred.
    pub fn first_after(&self, pattern_id: &str, ts: f64) -> R<Option<f64>> {
        let b: Option<i64> = self.conn().query_row(
            "SELECT MIN(bucket) FROM allc WHERE pattern_id = ?1 AND bucket > ?2",
            params![pattern_id, bucket_of(ts)],
            |r| r.get(0),
        )?;
        Ok(b.map(|b| b as f64 * BUCKET_SECS))
    }

    /// Hourly counts from `lo` on, per pattern, recent 10-second counts
    /// included, ordered by hour.
    pub fn hourly_counts(&self, lo: f64, mut f: impl FnMut(&str, i64, i64)) -> R<()> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT pattern_id, bucket / 360, SUM(n) FROM allc WHERE bucket >= ?1
             GROUP BY 2, 1 ORDER BY 2, 1",
        )?;
        let mut rows = st.query(params![bucket_of(lo) / 360 * 360])?;
        while let Some(r) = rows.next()? {
            f(&r.get::<_, String>(0)?, r.get(1)?, r.get(2)?);
        }
        Ok(())
    }

    /// Hourly field histograms from `lo` on: (pattern, field, hour, histogram).
    pub fn metric_rows(
        &self,
        lo: f64,
        mut f: impl FnMut(&str, &str, i64, &crate::metrics::Hist),
    ) -> R<()> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT pattern_id, field, hour, n, sum, min, max, bins FROM metric_hourly
             WHERE hour >= ?1 ORDER BY hour, pattern_id, field",
        )?;
        let mut rows = st.query(params![(lo / 3600.0).floor() as i64])?;
        while let Some(r) = rows.next()? {
            f(
                &r.get::<_, String>(0)?,
                &r.get::<_, String>(1)?,
                r.get(2)?,
                &hist_at(r, 3)?,
            );
        }
        Ok(())
    }

    /// Patterns seen at or after `lo`, oldest first.
    pub fn patterns_seen_since(&self, lo: f64) -> R<Vec<PatternRow>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(&format!(
            "SELECT {PATTERN_COLS} FROM patterns p WHERE p.last_seen >= ?1 ORDER BY p.first_seen"
        ))?;
        let rows = st.query_map(params![lo], |r| row_to_pattern(r, 0))?;
        rows.collect()
    }

    /// Patterns first seen in [lo, hi), oldest first.
    pub fn patterns_first_seen(&self, lo: f64, hi: f64) -> R<Vec<PatternRow>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(&format!(
            "SELECT {PATTERN_COLS} FROM patterns p WHERE p.first_seen >= ?1 AND p.first_seen < ?2 ORDER BY p.first_seen"
        ))?;
        let rows = st.query_map(params![lo, hi], |r| row_to_pattern(r, 0))?;
        rows.collect()
    }

    pub fn meta(&self, key: &str) -> R<Option<String>> {
        self.conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    pub fn set_meta(&self, key: &str, value: Option<&str>) -> R<()> {
        let conn = self.conn();
        match value {
            Some(v) => conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                params![key, v],
            )?,
            None => conn.execute("DELETE FROM meta WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// Number of patterns (merged ones count once).
    /// When the oldest pattern was first seen: roughly where the logs start.
    pub fn data_start(&self) -> R<Option<f64>> {
        self.conn()
            .query_row("SELECT MIN(first_seen) FROM patterns", [], |r| r.get(0))
    }

    pub fn pattern_count(&self) -> R<i64> {
        self.conn()
            .query_row("SELECT COUNT(*) FROM patterns", [], |r| r.get(0))
    }

    /// Patterns first seen at or after `since`.
    pub fn new_pattern_count(&self, since: f64) -> R<i64> {
        self.conn().query_row(
            "SELECT COUNT(*) FROM patterns WHERE first_seen >= ?1",
            [since],
            |r| r.get(0),
        )
    }

    /// Bytes the database takes on disk, with its write-ahead log; `None` in memory.
    pub fn disk_bytes(&self) -> R<Option<u64>> {
        let path: String = self
            .conn()
            .query_row("PRAGMA database_list", [], |r| r.get(2))?;
        if path.is_empty() {
            return Ok(None);
        }
        let size = |p: &str| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        Ok(Some(size(&path) + size(&format!("{path}-wal"))))
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
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO markers(name, ts, kind, note) VALUES (?1, ?2, ?3, ?4)",
            params![name, ts, kind, note],
        )?;
        if let Some(ob) = self
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let item = crate::push::Item::Marker {
                name: name.to_string(),
                at: ts,
                kind: kind.to_string(),
                note: note.map(str::to_string),
            };
            queue(&tx, ob, &item)?;
        }
        tx.commit()
    }

    /// Add a marker unless one with this name, kind and time exists (a hub
    /// hears about one deploy from every replica).
    pub fn add_marker_once(&self, name: &str, ts: f64, kind: &str, note: Option<&str>) -> R<bool> {
        let exists: bool = self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM markers WHERE name = ?1 AND kind = ?2 AND abs(ts - ?3) < 0.001)",
            params![name, kind, ts],
            |r| r.get(0),
        )?;
        if !exists {
            self.add_marker(name, ts, kind, note)?;
        }
        Ok(!exists)
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
