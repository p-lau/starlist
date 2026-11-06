//! Numbers read from log lines (`duration_ms=532`, `"latency": 0.53`), kept as
//! small per-pattern, per-hour histograms so percentiles survive for months
//! without storing the lines.

use crate::store::Store;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;

/// Bins per doubling. 8 gives about 9% relative error on a percentile.
const BINS_PER_OCTAVE: f64 = 8.0;
/// The bin for zero and negative values.
const NON_POSITIVE: i32 = i32::MIN;

/// A log-scale histogram: counts per bin plus exact count, sum, min and max.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hist {
    pub n: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub bins: BTreeMap<i32, u64>,
}

fn bin(v: f64) -> i32 {
    if v <= 0.0 {
        NON_POSITIVE
    } else {
        (v.log2() * BINS_PER_OCTAVE).floor() as i32
    }
}

fn bin_value(b: i32) -> f64 {
    if b == NON_POSITIVE {
        0.0
    } else {
        ((b as f64 + 0.5) / BINS_PER_OCTAVE).exp2()
    }
}

impl Hist {
    pub fn add(&mut self, v: f64) {
        if !v.is_finite() {
            return;
        }
        if self.n == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.n += 1;
        self.sum += v;
        *self.bins.entry(bin(v)).or_insert(0) += 1;
    }

    pub fn merge(&mut self, o: &Hist) {
        if o.n == 0 {
            return;
        }
        if self.n == 0 {
            self.min = o.min;
            self.max = o.max;
        } else {
            self.min = self.min.min(o.min);
            self.max = self.max.max(o.max);
        }
        self.n += o.n;
        self.sum += o.sum;
        for (b, c) in &o.bins {
            *self.bins.entry(*b).or_insert(0) += c;
        }
    }

    /// The value at quantile `q` (0 to 1), clamped to the observed range.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        if self.n == 0 {
            return None;
        }
        // The ends are known exactly.
        if q >= 1.0 {
            return Some(self.max);
        }
        if q <= 0.0 {
            return Some(self.min);
        }
        let rank = (q.clamp(0.0, 1.0) * self.n as f64).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (b, c) in &self.bins {
            seen += c;
            if seen >= rank {
                let v = bin_value(*b).clamp(self.min, self.max);
                return Some((v * 1000.0).round() / 1000.0);
            }
        }
        Some(self.max)
    }

    pub fn mean(&self) -> Option<f64> {
        (self.n > 0).then(|| (self.sum / self.n as f64 * 1000.0).round() / 1000.0)
    }

    pub fn summary(&self, field: &str) -> Option<Summary> {
        Some(Summary {
            field: field.to_string(),
            count: self.n,
            mean: self.mean()?,
            p50: self.quantile(0.5)?,
            p95: self.quantile(0.95)?,
            p99: self.quantile(0.99)?,
            min: self.min,
            max: self.max,
        })
    }

    /// Bins as "bin:count,bin:count" for storage.
    pub fn bins_text(&self) -> String {
        self.bins
            .iter()
            .map(|(b, c)| format!("{b}:{c}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn from_parts(n: u64, sum: f64, min: f64, max: f64, bins: &str) -> Self {
        let bins = bins
            .split(',')
            .filter_map(|p| {
                let (b, c) = p.split_once(':')?;
                Some((b.parse().ok()?, c.parse().ok()?))
            })
            .collect();
        Self {
            n,
            sum,
            min,
            max,
            bins,
        }
    }
}

/// Percentiles of one field. Percentiles are within about 9%; count, mean,
/// min and max are exact.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub field: String,
    pub count: u64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub min: f64,
    pub max: f64,
}

/// One histogram per field over the hours from `lo` to `hi`, for a pattern or all.
pub fn window(
    store: &Store,
    field: &str,
    pattern_id: Option<&str>,
    lo: f64,
    hi: f64,
) -> rusqlite::Result<Hist> {
    let mut h = Hist::default();
    for (_, x) in store.metric_hours(field, pattern_id, lo, hi)? {
        h.merge(&x);
    }
    Ok(h)
}

/// Every field recorded for a pattern (or any) over the hours from `lo` to
/// `hi`, most used first.
pub fn summaries(
    store: &Store,
    pattern_id: Option<&str>,
    lo: f64,
    hi: f64,
) -> rusqlite::Result<Vec<Summary>> {
    let mut out = vec![];
    for f in store.metric_fields(pattern_id)? {
        if let Some(s) = window(store, &f, pattern_id, lo, hi)?.summary(&f) {
            out.push(s);
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.count));
    Ok(out)
}

/// Which numeric fields to read from log lines.
#[derive(Debug, Clone)]
pub struct Fields(Vec<(String, Regex)>);

impl Fields {
    /// Field names like "duration_ms" or "latency". Matched in `key=1.5`,
    /// `key: 1.5` and JSON `"key": 1.5`, case-insensitively; a unit right after
    /// the number (`532ms`) is ignored.
    pub fn new(names: &[String]) -> Result<Self, regex::Error> {
        names
            .iter()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .map(|n| {
                let re = Regex::new(&format!(
                    r#"(?i)(?:^|[\s,{{(;"'])"?{}"?\s*[=:]\s*"?(-?\d+(?:\.\d+)?)"#,
                    regex::escape(n)
                ))?;
                Ok((n.to_string(), re))
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn extract(&self, text: &str) -> Vec<(String, f64)> {
        self.0
            .iter()
            .filter_map(|(name, re)| {
                let v: f64 = re.captures(text)?.get(1)?.as_str().parse().ok()?;
                Some((name.clone(), v))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_close() {
        let mut h = Hist::default();
        for i in 1..=1000 {
            h.add(i as f64);
        }
        let p50 = h.quantile(0.5).unwrap();
        let p99 = h.quantile(0.99).unwrap();
        assert!((p50 - 500.0).abs() / 500.0 < 0.1, "{p50}");
        assert!((p99 - 990.0).abs() / 990.0 < 0.1, "{p99}");
        assert_eq!(h.quantile(1.0), Some(1000.0));
        assert_eq!(h.mean(), Some(500.5));
        let back = Hist::from_parts(h.n, h.sum, h.min, h.max, &h.bins_text());
        assert_eq!(back, h);
        let mut z = Hist::default();
        z.add(0.0);
        z.add(-3.0);
        assert_eq!(z.quantile(0.5), Some(0.0));
    }

    #[test]
    fn reads_fields_in_common_shapes() {
        let f = Fields::new(&["duration_ms".into(), "latency".into()]).unwrap();
        assert_eq!(
            f.extract("GET /x 200 duration_ms=532"),
            vec![("duration_ms".to_string(), 532.0)]
        );
        assert_eq!(
            f.extract(r#"{"msg":"done","latency": 0.53,"x":1}"#),
            vec![("latency".to_string(), 0.53)]
        );
        assert_eq!(
            f.extract("took Latency: 12ms"),
            vec![("latency".to_string(), 12.0)]
        );
        assert!(f.extract("my_duration_ms=5 nolatency=3").is_empty());
    }
}
