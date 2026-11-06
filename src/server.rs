//! MCP surface: a handful of tools that answer from the local store.

use crate::analysis::{self, AResult};
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
This server watches one application's logs and answers with distilled, redacted summaries instead of raw lines. \
Log lines are grouped into patterns with stable IDs (the same kind of line keeps the same ID across restarts).
Typical flow:
1. whats_new: what appeared, spiked or returned recently compared with a baseline. Start here.
2. get_pattern: details, hourly counts and redacted samples for one pattern.
3. After shipping a fix, add_marker (or rely on a deploy marker detected from the logs), wait, then verify_fix(pattern_id, since=<marker>) to confirm the error stopped. It tells you when it's too early to say.
4. compare_deploy: everything that changed around a deploy, including new errors it may have introduced.
Times accept a duration back from now ('30m', '2h'), an ISO time, or a marker name.";

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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VerifyFixArgs {
    /// Pattern to check, from whats_new or list_patterns.
    pub pattern_id: String,
    /// When the fix went live: a marker name (e.g. "v1.4.3" or "fix-currency"), an ISO time, or a duration back from now like "20m".
    pub since: String,
    /// How far before the split to measure the old rate, like "2h". Default: as long as the time since the split, at least 1h.
    pub before: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDeployArgs {
    /// The deploy or change point: a marker name, an ISO time, or a duration back from now.
    pub at: String,
    /// How far before the split to compare against, like "2h". Default: as long as the time since the split, at least 1h.
    pub before: Option<String>,
    /// Maximum findings to return. Default 10.
    pub limit: Option<usize>,
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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPatternArgs {
    pub pattern_id: String,
    /// Number of redacted sample events to include, 1 to 5. Default 3.
    pub samples: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddMarkerArgs {
    /// Short unique name, e.g. "fix-currency-keyerror" or "v1.4.3".
    pub name: String,
    /// What changed, for later reference.
    pub note: Option<String>,
    /// When it happened, if not now: ISO time or a duration back from now.
    pub at: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListMarkersArgs {
    /// Maximum markers to return, newest first. Default 20.
    pub limit: Option<i64>,
}

#[derive(Clone)]
pub struct LogServer {
    store: Arc<Store>,
    tool_router: ToolRouter<Self>,
}

fn out(r: AResult) -> Result<String, String> {
    r.map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
}

#[tool_router]
impl LogServer {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store, tool_router: Self::tool_router() }
    }

    #[tool(
        description = "What's new in the logs: patterns that appeared for the first time, came back, spiked, or went quiet in the recent window compared with a baseline. Errors first. Start here.",
        annotations(read_only_hint = true)
    )]
    fn whats_new(&self, Parameters(a): Parameters<WhatsNewArgs>) -> Result<String, String> {
        out(analysis::whats_new(
            &self.store,
            a.window.as_deref().unwrap_or("1h"),
            a.baseline.as_deref().unwrap_or("24h"),
            a.min_level.as_deref(),
            a.limit.unwrap_or(10).clamp(1, 50),
            analysis::now(),
        ))
    }

    #[tool(
        description = "Check whether a log pattern (usually an error) stopped or dropped after a fix or deploy. Compares its rate before and after the split and returns a verdict: fixed, reduced, still_occurring, worse, too_early (with how long to wait), appeared_after, or not_active_before.",
        annotations(read_only_hint = true)
    )]
    fn verify_fix(&self, Parameters(a): Parameters<VerifyFixArgs>) -> Result<String, String> {
        out(analysis::verify_fix(&self.store, &a.pattern_id, &a.since, a.before.as_deref(), analysis::now()))
    }

    #[tool(
        description = "Everything that changed around a deploy or point in time: patterns new since then (possible regressions), increased, stopped, or decreased.",
        annotations(read_only_hint = true)
    )]
    fn compare_deploy(&self, Parameters(a): Parameters<CompareDeployArgs>) -> Result<String, String> {
        out(analysis::compare_deploy(
            &self.store,
            &a.at,
            a.before.as_deref(),
            a.limit.unwrap_or(10).clamp(1, 50),
            analysis::now(),
        ))
    }

    #[tool(
        description = "List log patterns seen in a period with their counts, most severe and most frequent first. Use to find a pattern ID or search by text.",
        annotations(read_only_hint = true)
    )]
    fn list_patterns(&self, Parameters(a): Parameters<ListPatternsArgs>) -> Result<String, String> {
        out(analysis::list_patterns(
            &self.store,
            a.since.as_deref().unwrap_or("24h"),
            a.min_level.as_deref(),
            a.contains.as_deref(),
            a.limit.unwrap_or(20).clamp(1, 100),
            analysis::now(),
        ))
    }

    #[tool(
        description = "Details for one pattern: full template, exception type, counts, hourly counts for the last 24h, recent markers, and redacted sample events (with stack traces).",
        annotations(read_only_hint = true)
    )]
    fn get_pattern(&self, Parameters(a): Parameters<GetPatternArgs>) -> Result<String, String> {
        out(analysis::get_pattern(&self.store, &a.pattern_id, a.samples.unwrap_or(3), analysis::now()))
    }

    #[tool(
        description = "Record a named point in time, such as when a fix was deployed, so verify_fix and compare_deploy can refer to it by name.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    fn add_marker(&self, Parameters(a): Parameters<AddMarkerArgs>) -> Result<String, String> {
        out(analysis::add_marker(&self.store, &a.name, a.note.as_deref(), a.at.as_deref(), analysis::now()))
    }

    #[tool(
        description = "List markers (deploys detected from the logs and markers added manually), newest first.",
        annotations(read_only_hint = true)
    )]
    fn list_markers(&self, Parameters(a): Parameters<ListMarkersArgs>) -> Result<String, String> {
        out(analysis::list_markers(&self.store, a.limit.unwrap_or(20), analysis::now()))
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
