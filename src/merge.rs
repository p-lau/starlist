//! Merging patterns that differ only in one word, such as a username without
//! digits ("user alice logged in", "user bob logged in", ...).
//!
//! Rule-based masking catches numbers and IDs, but not plain words. When the
//! same position takes at least `threshold` different values, the pattern is
//! generalized with `<*>` at that position. The first pattern's ID stays the
//! canonical ID, and the other IDs become aliases, so IDs an agent already
//! holds keep working.
//!
//! The threshold guards against merging lines that differ in meaning, like
//! "job started" / "job failed": a status word rarely has five values.

use crate::pattern::Event;
use crate::store::Store;
use std::collections::{HashMap, HashSet};

pub const DEFAULT_THRESHOLD: usize = 5;
const WILDCARD: &str = "<*>";

/// Patterns are only compared within a group of the same token count and first token.
type GroupKey = (usize, String);

struct Canon {
    id: String,
    tokens: Vec<String>,
}

/// One position of one canonical pattern where other patterns differ.
#[derive(Default)]
struct Variants {
    values: HashSet<String>,
    ids: Vec<String>,
}

/// What to change in the store when a merge happens.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeOp {
    pub canonical: String,
    pub template: String,
    pub aliases: Vec<String>,
}

pub struct Merger {
    threshold: usize,
    groups: HashMap<GroupKey, Vec<Canon>>,
    variants: HashMap<(String, usize), Variants>,
    aliases: HashMap<String, String>,
    templates: HashMap<String, String>,
}

fn tokens(t: &str) -> Vec<String> {
    t.split(' ').map(str::to_string).collect()
}

fn key(toks: &[String]) -> GroupKey {
    (toks.len(), toks.first().cloned().unwrap_or_default())
}

/// Keep a shared `key=` / `key:` prefix when generalizing: "user=alice" -> "user=<*>".
fn generalize(a: &str, b: &str) -> String {
    let cut = |s: &str| s.rfind(['=', ':']).map(|i| i + 1);
    match (cut(a), cut(b)) {
        (Some(i), Some(j)) if a[..i] == b[..j] && i < a.len() && j < b.len() => format!("{}{WILDCARD}", &a[..i]),
        _ => WILDCARD.to_string(),
    }
}

fn token_matches(canon: &str, tok: &str) -> bool {
    canon == tok
        || canon == WILDCARD
        || (canon.ends_with(WILDCARD) && {
            let prefix = &canon[..canon.len() - WILDCARD.len()];
            !prefix.is_empty() && tok.starts_with(prefix) && tok.len() > prefix.len()
        })
}

/// Positions where `toks` differs from `canon` (respecting wildcards).
fn diffs(canon: &[String], toks: &[String]) -> Vec<usize> {
    canon.iter().zip(toks).enumerate().filter(|(_, (c, t))| !token_matches(c, t)).map(|(i, _)| i).collect()
}

impl Merger {
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold: threshold.max(2),
            groups: HashMap::new(),
            variants: HashMap::new(),
            aliases: HashMap::new(),
            templates: HashMap::new(),
        }
    }

    /// Load existing patterns and aliases so merging continues across restarts.
    /// Merges that the stored patterns already qualify for are applied.
    pub fn load(threshold: usize, store: &Store) -> rusqlite::Result<Self> {
        let mut m = Self::new(threshold);
        m.aliases = store.aliases()?;
        for (id, template) in store.pattern_templates()? {
            if let Some(op) = m.observe_new(&id, &template) {
                store.merge_patterns(&op)?;
            }
        }
        Ok(m)
    }

    /// Point an event at its canonical pattern. Returns a merge to apply to the
    /// store when this event's pattern tips a position over the threshold.
    pub fn resolve(&mut self, e: &mut Event) -> Option<MergeOp> {
        if let Some(c) = self.aliases.get(&e.pattern_id).cloned() {
            self.retarget(e, &c);
            return None;
        }
        if self.templates.contains_key(&e.pattern_id) {
            return None;
        }
        let toks = tokens(&e.template);
        // Already covered by a generalized pattern?
        if let Some(c) = self.groups.get(&key(&toks)).and_then(|g| g.iter().find(|c| diffs(&c.tokens, &toks).is_empty())) {
            let cid = c.id.clone();
            self.aliases.insert(e.pattern_id.clone(), cid.clone());
            self.retarget(e, &cid);
            return None;
        }
        let op = self.observe_new(&e.pattern_id, &e.template);
        if let Some(c) = self.aliases.get(&e.pattern_id).cloned() {
            self.retarget(e, &c);
        }
        op
    }

    fn retarget(&self, e: &mut Event, canonical: &str) {
        e.pattern_id = canonical.to_string();
        if let Some(t) = self.templates.get(canonical) {
            e.template = t.clone();
        }
    }

    /// Register a pattern seen for the first time; may trigger a merge.
    fn observe_new(&mut self, id: &str, template: &str) -> Option<MergeOp> {
        let toks = tokens(template);
        let k = key(&toks);
        let group = self.groups.entry(k.clone()).or_default();
        let hit = group.iter().find_map(|c| {
            let d = diffs(&c.tokens, &toks);
            (d.len() == 1).then(|| (c.id.clone(), d[0]))
        });
        group.push(Canon { id: id.to_string(), tokens: toks.clone() });
        self.templates.insert(id.to_string(), template.to_string());

        let (cid, pos) = hit?;
        let canon_tok = self.groups[&k].iter().find(|c| c.id == cid)?.tokens[pos].clone();
        let v = self.variants.entry((cid.clone(), pos)).or_default();
        v.values.insert(canon_tok.clone());
        v.values.insert(toks[pos].clone());
        v.ids.push(id.to_string());
        if v.values.len() < self.threshold {
            return None;
        }

        // Merge: generalize the canonical pattern and fold the variants into it.
        let v = self.variants.remove(&(cid.clone(), pos))?;
        let general = v.values.iter().fold(canon_tok.clone(), |acc, t| if acc == *t { acc } else { generalize(&acc, t) });
        let group = self.groups.get_mut(&k)?;
        let canon = group.iter_mut().find(|c| c.id == cid)?;
        canon.tokens[pos] = general;
        let template = canon.tokens.join(" ");
        group.retain(|c| !v.ids.contains(&c.id));
        for a in &v.ids {
            self.templates.remove(a);
            self.aliases.insert(a.clone(), cid.clone());
        }
        // Aliases that pointed at a folded pattern now point at the canonical one.
        for target in self.aliases.values_mut() {
            if v.ids.contains(target) {
                *target = cid.clone();
            }
        }
        self.templates.insert(cid.clone(), template.clone());
        Some(MergeOp { canonical: cid, template, aliases: v.ids })
    }

    pub fn canonical_of(&self, id: &str) -> Option<&str> {
        self.aliases.get(id).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::parse_event;
    use crate::redact::Redactor;

    fn ev(s: &str) -> Event {
        parse_event(s, &Redactor::default(), Some(0.0), None)
    }

    #[test]
    fn merges_after_enough_distinct_values() {
        let mut m = Merger::new(5);
        let names = ["alice", "bob", "carol", "dave", "erin", "frank"];
        let mut ops = vec![];
        let mut first_id = String::new();
        for (i, n) in names.iter().enumerate() {
            let mut e = ev(&format!("INFO user {n} logged in"));
            if i == 0 {
                first_id = e.pattern_id.clone();
            }
            ops.extend(m.resolve(&mut e));
            if i >= 4 {
                assert_eq!(e.pattern_id, first_id, "event {i} should land on the canonical pattern");
            }
        }
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].template, "INFO user <*> logged in");
        assert_eq!(ops[0].canonical, first_id);
        assert_eq!(ops[0].aliases.len(), 4); // bob, carol, dave, erin
        let mut late = ev("INFO user zed logged in");
        assert!(m.resolve(&mut late).is_none());
        assert_eq!(late.pattern_id, first_id);
        assert_eq!(late.template, "INFO user <*> logged in");
    }

    #[test]
    fn keeps_status_words_apart() {
        let mut m = Merger::new(5);
        let mut ids = HashSet::new();
        for s in ["started", "stopped", "failed"] {
            let mut e = ev(&format!("INFO job sync {s}"));
            assert!(m.resolve(&mut e).is_none());
            ids.insert(e.pattern_id);
        }
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn keeps_key_prefix() {
        assert_eq!(generalize("user=alice", "user=bob"), "user=<*>");
        assert_eq!(generalize("alice", "bob"), "<*>");
        let mut m = Merger::new(3);
        let mut op = None;
        for n in ["alice", "bob", "carol"] {
            op = op.or(m.resolve(&mut ev(&format!("WARN login throttled user={n}"))));
        }
        assert_eq!(op.unwrap().template, "WARN login throttled user=<*>");
    }
}
