//! MCP surface: a handful of tools that answer from the local store.

use crate::analysis::{self, AResult, Detail, Page};
use crate::store::Store;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use std::sync::Arc;

const INSTRUCTIONS: &str = "\
Distilled, redacted view of one app's logs. Lines are grouped into patterns with stable IDs.
Flow: whats_new -> get_pattern (get_context for surrounding lines, get_request for one request's path) -> ship a fix -> add_marker (deploys are also auto-detected) -> verify_fix(pattern_id, since=marker).
Patterns show first_release, the deploy live when they first appeared.
Times: '30m', '2h', ISO time, or marker name. Output is brief by default; pass detail='full' for more.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WhatsNewArgs {
    /// Recent period to inspect, as a duration like "15m", "1h" or "1d". Default "1h".
    pub window: Option<String>,
    /// Period before the window used as "normal", like "24h". Default "24h".
    pub baseline: Option<String>,
    /// Only report patterns at or above this level: DEBUG, INFO, WARN, ERROR, FATAL.
    pub min_level: Option<String>,
    /// Maximum findings to return. Default 10.
    pub limit: Option<usize>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
    /// Continue a previous answer: pass its next_cursor unchanged.
    pub cursor: Option<String>,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VerifyFixArgs {
    /// Pattern to check, from whats_new or list_patterns.
    pub pattern_id: String,
    /// When the fix went live: a marker name (e.g. "v1.4.3" or "fix-currency"), an ISO time, or a duration back from now like "20m".
    pub since: String,
    /// How far before the split to measure the old rate, like "2h". Default: as long as the time since the split, at least 1h.
    pub before: Option<String>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDeployArgs {
    /// The deploy or change point: a marker name, an ISO time, or a duration back from now. Default: the latest deploy.
    pub at: Option<String>,
    /// How far before the split to compare against, like "2h". Default: as long as the time since the split, at least 1h.
    pub before: Option<String>,
    /// Maximum findings to return. Default 10.
    pub limit: Option<usize>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
    /// Continue a previous answer: pass its next_cursor unchanged.
    pub cursor: Option<String>,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListPatternsArgs {
    /// Start of the period, like "24h" (back from now), an ISO time or a marker name. Default "24h".
    pub since: Option<String>,
    /// Only patterns at or above this level.
    pub min_level: Option<String>,
    /// Case-insensitive text the pattern template must contain.
    pub contains: Option<String>,
    /// Maximum patterns to return. Default 20.
    pub limit: Option<usize>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
    /// Continue a previous answer: pass its next_cursor unchanged.
    pub cursor: Option<String>,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPatternArgs {
    pub pattern_id: String,
    /// Number of redacted sample events, 1 to 5. Default 1 (brief) or 3 (full).
    pub samples: Option<i64>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddMarkerArgs {
    /// Short unique name, e.g. "fix-currency-keyerror" or "v1.4.3".
    pub name: String,
    /// What changed, for later reference.
    pub note: Option<String>,
    /// When it happened, if not now: ISO time or a duration back from now.
    pub at: Option<String>,
    /// True if this is a release going live; patterns first seen after it are attributed to it.
    pub deploy: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRequestArgs {
    /// Request or trace ID, e.g. a finding's example_request. A full traceparent also works.
    pub request_id: String,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetContextArgs {
    /// Center on the latest occurrence of this pattern (at or before `at`, if given).
    pub pattern_id: Option<String>,
    /// Center on this time: a duration back from now like "5m", an ISO time or a marker name.
    pub at: Option<String>,
    /// Lines before the anchor, 0 to 200. Default 20.
    pub before: Option<i64>,
    /// Lines after the anchor, 0 to 200. Default 20.
    pub after: Option<i64>,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
    /// "brief" (default, compact) or "full" (absolute times, raw numbers, untrimmed samples).
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListMarkersArgs {
    /// Maximum markers to return, newest first. Default 20.
    pub limit: Option<i64>,
    /// Upper bound on the answer size in characters (min 1000). Default 8000 brief, 16000 full.
    pub max_chars: Option<usize>,
}

#[derive(Clone)]
pub struct LogServer {
    store: Arc<Store>,
    tool_router: ToolRouter<Self>,
}

fn out(r: AResult) -> Result<String, String> {
    r.map(|v| serde_json::to_string(&analysis::compact(v)).unwrap_or_default())
}

fn detail(d: &Option<String>) -> Result<Detail, String> {
    Detail::parse(d.as_deref())
}

fn budget(max_chars: Option<usize>, d: Detail) -> usize {
    max_chars
        .unwrap_or_else(|| Page::default_budget(d))
        .max(analysis::MIN_MAX_CHARS)
}

#[tool_router]
impl LogServer {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Start here. Patterns that are new, returning, spiking (beyond overall traffic) or gone quiet in a recent window vs a baseline. Errors first.",
        annotations(read_only_hint = true)
    )]
    fn whats_new(&self, Parameters(a): Parameters<WhatsNewArgs>) -> Result<String, String> {
        let d = detail(&a.detail)?;
        let (page, now) = Page::from_args(
            a.limit.unwrap_or(10).clamp(1, 50),
            a.cursor.as_deref(),
            a.max_chars,
            d,
            analysis::now(),
        )?;
        out(analysis::whats_new(
            &self.store,
            a.window.as_deref().unwrap_or("1h"),
            a.baseline.as_deref().unwrap_or("24h"),
            a.min_level.as_deref(),
            &page,
            d,
            now,
        ))
    }

    #[tool(
        description = "Did a pattern stop after a fix/deploy? Verdict: fixed, reduced, still_occurring, worse, too_early (+wait_minutes), appeared_after, not_active_before.",
        annotations(read_only_hint = true)
    )]
    fn verify_fix(&self, Parameters(a): Parameters<VerifyFixArgs>) -> Result<String, String> {
        out(analysis::verify_fix(
            &self.store,
            &a.pattern_id,
            &a.since,
            a.before.as_deref(),
            detail(&a.detail)?,
            analysis::now(),
        ))
    }

    #[tool(
        description = "What changed around a deploy: new_after (likely regressions), increased, stopped, decreased.",
        annotations(read_only_hint = true)
    )]
    fn compare_deploy(
        &self,
        Parameters(a): Parameters<CompareDeployArgs>,
    ) -> Result<String, String> {
        let d = detail(&a.detail)?;
        let (page, now) = Page::from_args(
            a.limit.unwrap_or(10).clamp(1, 50),
            a.cursor.as_deref(),
            a.max_chars,
            d,
            analysis::now(),
        )?;
        out(analysis::compare_deploy(
            &self.store,
            a.at.as_deref(),
            a.before.as_deref(),
            &page,
            d,
            now,
        ))
    }

    #[tool(
        description = "Find patterns by text or level, with counts. Most severe, then most frequent first.",
        annotations(read_only_hint = true)
    )]
    fn list_patterns(&self, Parameters(a): Parameters<ListPatternsArgs>) -> Result<String, String> {
        let d = detail(&a.detail)?;
        let (page, now) = Page::from_args(
            a.limit.unwrap_or(20).clamp(1, 100),
            a.cursor.as_deref(),
            a.max_chars,
            d,
            analysis::now(),
        )?;
        out(analysis::list_patterns(
            &self.store,
            a.since.as_deref().unwrap_or("24h"),
            a.min_level.as_deref(),
            a.contains.as_deref(),
            &page,
            d,
            now,
        ))
    }

    #[tool(
        description = "One pattern: template, exception, counts, 24h hourly counts, redacted samples with app stack frames.",
        annotations(read_only_hint = true)
    )]
    fn get_pattern(&self, Parameters(a): Parameters<GetPatternArgs>) -> Result<String, String> {
        out(analysis::get_pattern(
            &self.store,
            &a.pattern_id,
            a.samples,
            budget(a.max_chars, detail(&a.detail)?),
            detail(&a.detail)?,
            analysis::now(),
        ))
    }

    #[tool(
        description = "The redacted raw lines just before and after an occurrence of a pattern or a point in time, from a buffer of recent events.",
        annotations(read_only_hint = true)
    )]
    fn get_context(&self, Parameters(a): Parameters<GetContextArgs>) -> Result<String, String> {
        let d = detail(&a.detail)?;
        out(analysis::get_context(
            &self.store,
            a.pattern_id.as_deref(),
            a.at.as_deref(),
            a.before.unwrap_or(20),
            a.after.unwrap_or(20),
            budget(a.max_chars, d),
            d,
            analysis::now(),
        ))
    }

    #[tool(
        description = "Every event of one request, in order, redacted: the path that led to an error. Findings carry example_request IDs.",
        annotations(read_only_hint = true)
    )]
    fn get_request(&self, Parameters(a): Parameters<GetRequestArgs>) -> Result<String, String> {
        let d = detail(&a.detail)?;
        out(analysis::get_request(
            &self.store,
            &a.request_id,
            budget(a.max_chars, d),
            d,
            analysis::now(),
        ))
    }

    #[tool(
        description = "Name a point in time (e.g. a fix going live) for verify_fix/compare_deploy.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn add_marker(&self, Parameters(a): Parameters<AddMarkerArgs>) -> Result<String, String> {
        out(analysis::add_marker(
            &self.store,
            &a.name,
            a.note.as_deref(),
            a.at.as_deref(),
            a.deploy.unwrap_or(false),
            analysis::now(),
        ))
    }

    #[tool(
        description = "Markers (auto-detected deploys and manual), newest first.",
        annotations(read_only_hint = true)
    )]
    fn list_markers(&self, Parameters(a): Parameters<ListMarkersArgs>) -> Result<String, String> {
        out(analysis::list_markers(
            &self.store,
            a.limit.unwrap_or(20),
            budget(a.max_chars, Detail::Brief),
            analysis::now(),
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LogServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("starlist", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
