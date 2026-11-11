use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use regex::Regex;
use starlist_ci::ingest::Engine;
use starlist_ci::report::{self, Shape};
use starlist_core::pattern::{level_rank, normalize_level};
use starlist_core::redact::Redactor;
use std::fs::File;
use std::io::{BufReader, Write};

/// Compare a failing CI run's log with the last passing run's, and report
/// what is new, what grew and what stopped, in a few hundred tokens.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// The failing run's log; `-` or nothing reads stdin.
    #[arg(default_value = "-")]
    failing: String,

    /// The last passing run's log. Without it, the failing log's errors and
    /// warnings are listed instead.
    #[arg(long, short)]
    baseline: Option<String>,

    /// `json` for agents, `markdown` for people (a job summary or PR comment).
    #[arg(long, value_enum, default_value_t = Format::Json)]
    format: Format,

    /// `full` adds a sample to every finding and longer templates.
    #[arg(long, value_enum, default_value_t = Detail::Brief)]
    detail: Detail,

    /// Most findings to list [default: 10 brief, 30 full].
    #[arg(long)]
    limit: Option<usize>,

    /// Answer size limit in characters, as for the MCP tools [default: 8000 brief, 16000 full].
    #[arg(long)]
    max_chars: Option<usize>,

    /// Leave out findings below this level (DEBUG, INFO, WARN, ERROR, FATAL)
    /// [default: none with a baseline, WARN without].
    #[arg(long)]
    min_level: Option<String>,

    /// Strip this prefix from code locations; repeatable.
    #[arg(long, env = "GITHUB_WORKSPACE")]
    source_root: Vec<String>,

    /// Also redact text matching this regex; repeatable.
    #[arg(long)]
    redact: Vec<String>,

    /// Also redact IP addresses.
    #[arg(long)]
    redact_ips: bool,

    /// Merge patterns that differ in one word once it takes this many values; 0 turns merging off.
    #[arg(long, default_value_t = starlist_core::merge::DEFAULT_THRESHOLD)]
    merge_after: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Markdown,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Detail {
    Brief,
    Full,
}

fn open(path: &str) -> Result<Box<dyn std::io::BufRead>> {
    Ok(if path == "-" {
        Box::new(BufReader::new(std::io::stdin().lock()))
    } else {
        Box::new(BufReader::new(
            File::open(path).with_context(|| format!("can't open {path}"))?,
        ))
    })
}

fn main() -> Result<()> {
    let a = Args::parse();
    if a.baseline.as_deref() == Some("-") && a.failing == "-" {
        bail!("only one of the logs can come from stdin");
    }
    let extra = a
        .redact
        .iter()
        .map(|r| Regex::new(r).with_context(|| format!("bad --redact pattern {r}")))
        .collect::<Result<Vec<_>>>()?;
    let min_severity = match &a.min_level {
        Some(l) => match normalize_level(l) {
            Some(l) => level_rank(Some(l)),
            None => bail!("unknown level '{l}'; use DEBUG, INFO, WARN, ERROR or FATAL"),
        },
        None if a.baseline.is_none() => level_rank(Some("WARN")),
        None => 0,
    };
    let full = a.detail == Detail::Full;
    let shape = Shape {
        full,
        limit: a.limit.unwrap_or(if full { 30 } else { 10 }).max(1),
        max_chars: a
            .max_chars
            .unwrap_or(if full {
                report::DEFAULT_MAX_CHARS_FULL
            } else {
                report::DEFAULT_MAX_CHARS
            })
            .max(report::MIN_MAX_CHARS),
    };
    let merge_after = if a.merge_after == 0 {
        usize::MAX
    } else {
        a.merge_after
    };
    let roots: Vec<String> = a
        .source_root
        .into_iter()
        .filter(|r| !r.is_empty())
        .collect();
    let mut engine = Engine::new(Redactor::new(a.redact_ips, extra), roots, merge_after);

    // The passing run goes first, so a pattern keeps the ID it had there.
    let mut passing = match &a.baseline {
        Some(p) => Some(
            engine
                .read(open(p)?)
                .with_context(|| format!("can't read {p}"))?,
        ),
        None => None,
    };
    let mut failing = engine
        .read(open(&a.failing)?)
        .with_context(|| format!("can't read {}", a.failing))?;
    if let Some(p) = passing.as_mut() {
        engine.settle(p);
    }
    engine.settle(&mut failing);

    let findings = report::compare(&failing, passing.as_ref(), min_severity);
    let v = report::to_json(&failing, passing.as_ref(), &findings, shape);
    let text = match a.format {
        Format::Json => serde_json::to_string(&v)?,
        Format::Markdown => report::to_markdown(&v),
    };
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", text.trim_end())?;
    Ok(())
}
