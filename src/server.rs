//! MCP surface: a handful of tools that answer from the local store.

use crate::alerts::{Alerts, RESOURCE_URI};
use crate::analysis::{self, AResult, Detail, Page};
use crate::store::Store;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Implementation, ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, Resource, ResourceContents,
        ResourceUpdatedNotificationParam, ServerCapabilities, ServerConfig, SubscribeRequestParams,
        SubscriptionFilter, UnsubscribeRequestParams,
    },
    schemars,
    service::{RequestContext, SubscriptionContext},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

const INSTRUCTIONS: &str = "\
Runtime evidence from the app's logs, as redacted patterns with stable IDs. If a stack trace or clear symptom already points at recent code, read the code first. Use starlist when the symptom is unknown, nothing was deployed, the codebase is large, or the cause may be outside the code (upstream, data, config, clients); prefer it over raw log files.
Start with diagnose; whats_new lists every change, get_pattern and get_request go deeper. After a fix: verify_fix, then set_pattern_state resolved.
Times: '30m', ISO time or a marker (deploys are detected). Optional on most tools: detail 'full', max_chars, and cursor (a previous next_cursor).";

/// The tools `--tools core` keeps: enough to find a problem and check a fix.
pub const CORE_TOOLS: &[&str] = &[
    "diagnose",
    "whats_new",
    "get_pattern",
    "get_context",
    "get_request",
    "verify_fix",
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WhatsNewArgs {
    /// Recent period, like "15m" or "1h". Default: since the latest deploy if it was in the last 6h, else "1h".
    pub window: Option<String>,
    /// Period before it taken as normal. Default "24h".
    pub baseline: Option<String>,
    /// DEBUG, INFO, WARN, ERROR or FATAL.
    pub min_level: Option<String>,
    /// Default 10.
    pub limit: Option<usize>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
    /// A previous answer's next_cursor.
    pub cursor: Option<String>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DiagnoseArgs {
    /// Like "1h". Default: since a deploy in the last 6h, else 1h.
    pub window: Option<String>,
    pub min_level: Option<String>,
    /// Causes to explain. Default 3.
    pub limit: Option<usize>,
    pub detail: Option<String>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VerifyFixArgs {
    pub pattern_id: String,
    /// When the fix went live: marker, ISO time or "20m".
    pub since: String,
    /// Period before the split to compare with. Default: as long as since the split, at least 1h.
    pub before: Option<String>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDeployArgs {
    /// Marker, ISO time or "30m". Default: the latest deploy.
    pub at: Option<String>,
    /// Period before the split to compare with. Default: as long as since the split, at least 1h.
    pub before: Option<String>,
    /// Default 10.
    pub limit: Option<usize>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
    /// A previous answer's next_cursor.
    pub cursor: Option<String>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListPatternsArgs {
    /// Default "24h".
    pub since: Option<String>,
    pub min_level: Option<String>,
    /// Text the template contains (any case).
    pub contains: Option<String>,
    /// Default 20.
    pub limit: Option<usize>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
    /// A previous answer's next_cursor.
    pub cursor: Option<String>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPatternArgs {
    pub pattern_id: String,
    /// 1 to 5. Default 1 (brief) or 3 (full).
    pub samples: Option<i64>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddMarkerArgs {
    /// Like "fix-currency" or "v1.4.3".
    pub name: String,
    pub note: Option<String>,
    /// If not now: ISO time or "5m".
    pub at: Option<String>,
    /// A release going live.
    pub deploy: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRequestArgs {
    /// Like a finding's example_request; a traceparent works too.
    pub request_id: String,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetContextArgs {
    /// Center on its latest occurrence (before `at`, if given).
    pub pattern_id: Option<String>,
    /// Center on this time.
    pub at: Option<String>,
    /// Lines before, 0 to 200. Default 20.
    pub before: Option<i64>,
    /// Lines after, 0 to 200. Default 20.
    pub after: Option<i64>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetPatternStateArgs {
    pub pattern_id: String,
    /// resolved, resolved_next (from the next deploy), ignored or open.
    pub state: Option<String>,
    /// usual (expected at set times, like a nightly job: its spikes aren't alerted unless
    /// worse than earlier days), always_alert, or auto (judge from history, the default).
    pub timing: Option<String>,
    /// With ignored: for this long, like "24h".
    pub ignore_for: Option<String>,
    /// With ignored: for this many more events.
    pub ignore_count: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetReleaseArgs {
    /// Default: the live release.
    pub name: Option<String>,
    /// "brief" (default) or "full".
    pub detail: Option<String>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MergePatternsArgs {
    /// The first is kept; the others resolve to it.
    pub pattern_ids: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SplitPatternArgs {
    /// The merged pattern, or an ID that was merged into it.
    pub pattern_id: String,
    /// Merged-in IDs to split out. Default: the `pattern_id` itself if it was
    /// merged in, otherwise all of them.
    pub pattern_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListMarkersArgs {
    /// Default 20.
    pub limit: Option<i64>,
    /// Answer size limit in characters.
    pub max_chars: Option<usize>,
}

#[derive(Clone)]
pub struct LogServer {
    store: Arc<Store>,
    tool_router: ToolRouter<Self>,
    alerts: Option<Arc<Alerts>>,
    /// Sessions that used the older `resources/subscribe`.
    legacy_watch: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
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
            alerts: None,
            legacy_watch: Arc::new(Mutex::new(None)),
        }
    }

    /// The tools, as listed to clients.
    pub fn new_tools() -> Vec<rmcp::model::Tool> {
        Self::tool_router()
            .list_all()
            .into_iter()
            .map(compact_tool)
            .collect()
    }

    /// Every tool's name.
    pub fn tool_names() -> Vec<String> {
        Self::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
    }

    /// Offer only these tools (`None` keeps them all). Each tool definition is sent with
    /// every model call, so a smaller set costs an agent less per turn.
    pub fn with_tools(mut self, keep: Option<&[String]>) -> Self {
        if let Some(keep) = keep {
            for name in Self::tool_names() {
                if !keep.contains(&name) {
                    self.tool_router.remove_route(&name);
                }
            }
        }
        self
    }

    /// Let clients subscribe to the whats_new resource and get alerts.
    pub fn with_alerts(mut self, alerts: Option<Arc<Alerts>>) -> Self {
        self.alerts = alerts;
        self
    }

    /// The resource body: whats_new for the last hour plus the latest alert.
    fn whats_new_resource(&self) -> Result<String, String> {
        let now = analysis::now();
        let mut v = analysis::whats_new(
            &self.store,
            None,
            None,
            None,
            &Page::first(10, Detail::Brief),
            Detail::Brief,
            now,
        )?;
        if let Some(a) = self.alerts.as_ref().and_then(|a| a.latest()) {
            v["latest_alert"] = a.to_json(now);
        }
        out(Ok(v))
    }

    #[tool(
        description = "Start here. Likely causes of what is wrong now, ranked: why each stands out (new, spiking, slowly rising...), the deploy it started with, code location, sample lines.",
        annotations(read_only_hint = true)
    )]
    fn diagnose(&self, Parameters(a): Parameters<DiagnoseArgs>) -> Result<String, String> {
        let d = detail(&a.detail)?;
        out(crate::diagnose::diagnose(
            &self.store,
            a.window.as_deref(),
            a.min_level.as_deref(),
            a.limit.unwrap_or(3).clamp(1, 10),
            budget(a.max_chars, d),
            d,
            analysis::now(),
        ))
    }

    #[tool(
        description = "Every pattern that is new, returning, spiking, rising or gone quiet in a window vs a baseline.",
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
            a.window.as_deref(),
            a.baseline.as_deref(),
            a.min_level.as_deref(),
            &page,
            d,
            now,
        ))
    }

    #[tool(
        description = "Did a pattern stop after a fix or deploy? Gives a verdict (fixed, reduced, still_occurring, too_early...).",
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
        description = "What changed around a deploy: new, increased, stopped, decreased.",
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
        description = "Find patterns by text or level, with counts.",
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
        description = "One pattern: counts, hourly history, code location, samples with stack frames.",
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
        description = "Raw lines around an occurrence of a pattern or a time.",
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
        description = "Every event of one request, in order.",
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
        description = "Name a point in time, like a fix going live.",
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
        description = "Set a pattern's state or timing. A resolved pattern that occurs again is reported as regressed.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn set_pattern_state(
        &self,
        Parameters(a): Parameters<SetPatternStateArgs>,
    ) -> Result<String, String> {
        out(analysis::set_pattern_state(
            &self.store,
            &a.pattern_id,
            a.state.as_deref(),
            a.timing.as_deref(),
            a.ignore_for.as_deref(),
            a.ignore_count,
            a.note.as_deref(),
            analysis::now(),
        ))
    }

    #[tool(
        description = "What a release changed, and its error rate vs the previous one.",
        annotations(read_only_hint = true)
    )]
    fn get_release(&self, Parameters(a): Parameters<GetReleaseArgs>) -> Result<String, String> {
        let d = detail(&a.detail)?;
        out(analysis::get_release(
            &self.store,
            a.name.as_deref(),
            budget(a.max_chars, d),
            d,
            analysis::now(),
        ))
    }

    #[tool(
        description = "Merge patterns that are the same issue.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    fn merge_patterns(
        &self,
        Parameters(a): Parameters<MergePatternsArgs>,
    ) -> Result<String, String> {
        out(analysis::merge_patterns(&self.store, &a.pattern_ids))
    }

    #[tool(
        description = "Undo a merge: split patterns back out of a merged one.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    fn split_pattern(&self, Parameters(a): Parameters<SplitPatternArgs>) -> Result<String, String> {
        out(analysis::split_pattern(
            &self.store,
            &a.pattern_id,
            a.pattern_ids.as_deref(),
        ))
    }

    #[tool(
        description = "Deploys and other markers, newest first.",
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

/// Tool definitions are sent with every model call an agent makes, so they are kept small:
/// no `$schema`, plain types instead of `["string", "null"]`, no number formats, and the
/// options every tool shares (detail, max_chars, cursor) explained once in the instructions.
fn compact_tool(mut tool: rmcp::model::Tool) -> rmcp::model::Tool {
    use serde_json::Value;
    let mut schema = (*tool.input_schema).clone();
    schema.remove("$schema");
    if let Some(Value::Object(props)) = schema.get_mut("properties") {
        for (name, prop) in props.iter_mut() {
            let Value::Object(prop) = prop else { continue };
            if let Some(Value::Array(types)) = prop.get("type") {
                let kept: Vec<Value> = types.iter().filter(|t| *t != "null").cloned().collect();
                if kept.len() == 1 {
                    prop.insert("type".into(), kept[0].clone());
                }
            }
            prop.remove("format");
            prop.remove("minimum");
            if matches!(name.as_str(), "detail" | "max_chars" | "cursor") {
                prop.remove("description");
            }
        }
    }
    tool.input_schema = Arc::new(schema);
    tool
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LogServer {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, McpError> {
        Ok(rmcp::model::ListToolsResult {
            tools: self
                .tool_router
                .list_all()
                .into_iter()
                .map(compact_tool)
                .collect(),
            ..Default::default()
        })
    }

    fn get_info(&self) -> ServerConfig {
        let caps = if self.alerts.is_some() {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .build()
        } else {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build()
        };
        ServerConfig::new(caps)
            .with_server_info(Implementation::new("starlist", env!("CARGO_PKG_VERSION")))
            .with_instructions(if self.tool_router.has_route("set_pattern_state") {
                INSTRUCTIONS.to_string()
            } else {
                INSTRUCTIONS.replace(", then set_pattern_state resolved", "")
            })
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new(RESOURCE_URI, "whats_new")
                .with_description(
                    "whats_new for the last hour, plus the latest alert. Subscribe to hear about new or spiking errors within seconds.",
                )
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        if request.uri != RESOURCE_URI {
            return Err(McpError::resource_not_found(
                format!(
                    "Unknown resource '{}'. The only resource is {RESOURCE_URI}.",
                    request.uri
                ),
                None,
            ));
        }
        let text = self
            .whats_new_resource()
            .map_err(|e| McpError::internal_error(e, None))?;
        let mut contents = ResourceContents::text(text, RESOURCE_URI);
        if let ResourceContents::TextResourceContents { mime_type, .. } = &mut contents {
            *mime_type = Some("application/json".into());
        }
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        self.alerts.as_ref()?;
        let wants = requested
            .resource_subscriptions
            .as_ref()
            .is_some_and(|u| u.iter().any(|u| u == RESOURCE_URI));
        Some(if wants {
            SubscriptionFilter::builder()
                .resource_subscription(RESOURCE_URI)
                .build()
        } else {
            SubscriptionFilter::new()
        })
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        let subscribed = context
            .accepted()
            .resource_subscriptions
            .as_ref()
            .is_some_and(|u| u.iter().any(|u| u == RESOURCE_URI));
        let Some(alerts) = self.alerts.as_ref().filter(|_| subscribed) else {
            context.cancelled().await;
            return Ok(());
        };
        let mut rx = alerts.subscribe();
        loop {
            tokio::select! {
                _ = context.cancelled() => return Ok(()),
                msg = rx.recv() => match msg {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if context.sink().notify_resource_updated(RESOURCE_URI).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                },
            }
        }
    }

    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let Some(alerts) = &self.alerts else {
            return Err(McpError::invalid_request(
                "Alerts are off for this server.",
                None,
            ));
        };
        if request.uri != RESOURCE_URI {
            return Err(McpError::resource_not_found(
                format!("Only {RESOURCE_URI} can be subscribed to."),
                None,
            ));
        }
        let mut rx = alerts.subscribe();
        let peer = context.peer.clone();
        let task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let p = ResourceUpdatedNotificationParam::new(RESOURCE_URI);
                        if peer.notify_resource_updated(p).await.is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        });
        let mut slot = self.legacy_watch.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = slot.replace(task) {
            old.abort();
        }
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        _request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        if let Some(t) = self
            .legacy_watch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            t.abort();
        }
        Ok(())
    }
}
