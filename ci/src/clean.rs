//! Turning CI runner output back into plain log lines.
//!
//! CI logs wrap each line in runner noise: color codes, a runner timestamp,
//! `##[group]` markers, and, in `gh run view --log` output, the job and step
//! name. That noise is removed here, and the step (or group) a line belongs to
//! is kept so a finding can say where in the job it happened.

use regex::Regex;
use std::sync::LazyLock;

/// Longest line kept; the rest of a longer line is cut off.
const MAX_LINE_CHARS: usize = 4000;

static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").unwrap()
});
/// `gh run view --log`: job, step, then the runner's timestamp.
static GH_COLUMNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[^\t]*\t([^\t]*)\t(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z) ?").unwrap()
});
/// A runner timestamp at the start of a raw job log line.
static RUNNER_TS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z ").unwrap());
/// `::error file=a.rs,line=3::message` and the like, as echoed into a log.
static WORKFLOW_COMMAND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^::(error|warning|notice|debug)(?: [^:]*)?::").unwrap());

/// What a raw line turned into.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// A log line, cleaned.
    Text(String),
    /// A step or group starts here; the line itself is not log output.
    Section(String),
    /// Runner bookkeeping with nothing to say.
    Skip,
}

/// Cleans CI output line by line and remembers the current step.
#[derive(Default)]
pub struct Cleaner {
    section: Option<String>,
    /// Steps come from `gh run view --log` columns, so groups don't replace them.
    has_columns: bool,
}

impl Cleaner {
    /// The step or group the last line belonged to.
    pub fn section(&self) -> Option<&str> {
        self.section.as_deref()
    }

    pub fn clean(&mut self, raw: &str) -> Line {
        let mut s = raw
            .trim_end_matches(['\n', '\r'])
            .trim_start_matches('\u{feff}');
        let mut owned;
        if let Some(c) = GH_COLUMNS.captures(s) {
            self.has_columns = true;
            let step = c[1].trim();
            if !step.is_empty() && self.section.as_deref() != Some(step) {
                self.section = Some(step.to_string());
            }
            s = &s[c.get(0).unwrap().end()..];
        } else if let Some(m) = RUNNER_TS.find(s) {
            s = &s[m.end()..];
        }
        if s.contains('\x1b') {
            owned = ANSI.replace_all(s, "").into_owned();
            s = &owned;
        }

        if let Some(name) = s
            .strip_prefix("##[group]")
            .or_else(|| s.strip_prefix("::group::"))
        {
            let name = name.trim();
            if self.has_columns || name.is_empty() {
                return Line::Skip;
            }
            self.section = Some(name.to_string());
            return Line::Section(name.to_string());
        }
        if s.starts_with("##[endgroup]") || s.starts_with("::endgroup::") {
            return Line::Skip;
        }
        owned = if let Some(rest) = s.strip_prefix("##[error]") {
            format!("ERROR {rest}")
        } else if let Some(rest) = s.strip_prefix("##[warning]") {
            format!("WARN {rest}")
        } else if let Some(rest) = ["##[notice]", "##[debug]", "##[command]", "##[section]"]
            .iter()
            .find_map(|p| s.strip_prefix(p))
        {
            rest.to_string()
        } else if let Some(c) = WORKFLOW_COMMAND.captures(s) {
            let level = match &c[1] {
                "error" => "ERROR ",
                "warning" => "WARN ",
                _ => "",
            };
            format!("{level}{}", &s[c.get(0).unwrap().end()..])
        } else {
            s.to_string()
        };
        if owned.len() > MAX_LINE_CHARS {
            let mut cut = MAX_LINE_CHARS;
            while !owned.is_char_boundary(cut) {
                cut -= 1;
            }
            owned.truncate(cut);
        }
        Line::Text(owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(c: &mut Cleaner, s: &str) -> String {
        match c.clean(s) {
            Line::Text(t) => t,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn strips_runner_timestamp_and_colors() {
        let mut c = Cleaner::default();
        assert_eq!(
            text(
                &mut c,
                "2026-11-10T21:54:09.1234567Z \x1b[1m\x1b[31merror\x1b[0m: could not compile"
            ),
            "error: could not compile"
        );
    }

    #[test]
    fn keeps_an_apps_own_timestamp() {
        let mut c = Cleaner::default();
        assert_eq!(
            text(&mut c, "2026-11-10T21:54:09Z ERROR boom"),
            "2026-11-10T21:54:09Z ERROR boom"
        );
    }

    #[test]
    fn gh_run_view_columns_give_the_step() {
        let mut c = Cleaner::default();
        let t = text(
            &mut c,
            "build\tRun cargo test\t2026-11-10T21:54:09.1234567Z test a ... ok",
        );
        assert_eq!(t, "test a ... ok");
        assert_eq!(c.section(), Some("Run cargo test"));
        // Groups inside a step don't replace it.
        assert_eq!(
            c.clean("build\tRun cargo test\t2026-11-10T21:54:09.1234567Z ##[group]inner"),
            Line::Skip
        );
        assert_eq!(c.section(), Some("Run cargo test"));
    }

    #[test]
    fn groups_and_annotations() {
        let mut c = Cleaner::default();
        assert_eq!(
            c.clean("##[group]Run cargo test"),
            Line::Section("Run cargo test".into())
        );
        assert_eq!(c.section(), Some("Run cargo test"));
        assert_eq!(c.clean("##[endgroup]"), Line::Skip);
        assert_eq!(
            text(&mut c, "##[error]Process completed with exit code 101."),
            "ERROR Process completed with exit code 101."
        );
        assert_eq!(
            text(&mut c, "::warning file=src/a.rs,line=3::unused variable"),
            "WARN unused variable"
        );
    }
}
