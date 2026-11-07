//! The hub (`starlist hub`): the ground station for many collectors.
//!
//! Collectors push what they record to `/hub/v1/push` (see `push`). The hub
//! keeps one database per service in its directory, so replicas of a service
//! add up in one place (pattern IDs are the same on every host), and every
//! MCP tool and API route works per service unchanged. On top of that it
//! answers across services: `get_request` follows one request through all of
//! them, and `whats_new` without a service covers each one.

use crate::analysis::{self, Detail, ReqLine, db};
use crate::push::{Ack, Batch, Item, valid_service};
use crate::server::LogServer;
use crate::store::Store;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

/// File in the hub directory with one `service sha1(token)` line per collector token.
const TOKENS_FILE: &str = "collectors.tokens";
/// Most events read per service for one request.
const REQUEST_LINES: i64 = 2000;

pub struct Hub {
    dir: PathBuf,
    /// Raw-line buffer per service; `None` keeps no raw lines.
    raw_cap: Option<i64>,
    services: RwLock<BTreeMap<String, Arc<Store>>>,
    apis: RwLock<BTreeMap<String, Router>>,
}

fn hash(token: &str) -> String {
    sha1_smol::Sha1::from(token.trim().as_bytes())
        .digest()
        .to_string()
}

impl Hub {
    /// Open the hub directory, creating it if needed, with every service
    /// database already in it.
    pub fn open(dir: impl AsRef<Path>, raw_cap: Option<i64>) -> Result<Self, String> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let hub = Self {
            dir,
            raw_cap,
            services: RwLock::default(),
            apis: RwLock::default(),
        };
        let entries = std::fs::read_dir(&hub.dir)
            .map_err(|e| format!("reading {}: {e}", hub.dir.display()))?;
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "db")
                && let Some(name) = p.file_stem().and_then(|s| s.to_str())
                && valid_service(name)
            {
                hub.store_or_create(name)?;
            }
        }
        Ok(hub)
    }

    /// Services that have pushed, in name order.
    pub fn names(&self) -> Vec<String> {
        self.services
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn store(&self, service: &str) -> Option<Arc<Store>> {
        self.services
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(service)
            .cloned()
    }

    pub fn stores(&self) -> Vec<(String, Arc<Store>)> {
        self.services
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn store_or_create(&self, service: &str) -> Result<Arc<Store>, String> {
        if !valid_service(service) {
            return Err(format!(
                "'{service}' is not a service name; use letters, digits, '-', '_' and '.'."
            ));
        }
        if let Some(s) = self.store(service) {
            return Ok(s);
        }
        let mut map = self.services.write().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = map.get(service) {
            return Ok(s.clone());
        }
        let path = self.dir.join(format!("{service}.db"));
        let store =
            Arc::new(Store::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?);
        store.set_raw_cap(self.raw_cap).map_err(db)?;
        map.insert(service.to_string(), store.clone());
        Ok(store)
    }

    /// The service whose database has this pattern (or an alias of it).
    pub fn find_pattern(&self, pattern_id: &str) -> Option<String> {
        self.stores()
            .into_iter()
            .find(|(_, s)| s.pattern(pattern_id).ok().flatten().is_some())
            .map(|(n, _)| n)
    }

    /// The JSON API router for one service.
    fn api(&self, service: &str) -> Option<Router> {
        if let Some(r) = self
            .apis
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(service)
        {
            return Some(r.clone());
        }
        let store = self.store(service)?;
        let r = crate::api::routes(crate::api::ApiState {
            store,
            alerts: None,
            service: Some(service.to_string()),
        });
        self.apis
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(service.to_string(), r.clone());
        Some(r)
    }

    /// Create a push token for a service and save its hash. Returns the token.
    pub fn add_token(dir: impl AsRef<Path>, service: &str) -> Result<String, String> {
        if !valid_service(service) {
            return Err(format!(
                "'{service}' is not a service name; use letters, digits, '-', '_' and '.'."
            ));
        }
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let token = crate::http::generate_token().map_err(|e| e.to_string())?;
        let path = dir.join(TOKENS_FILE);
        let mut opts = std::fs::OpenOptions::new();
        opts.append(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        use std::io::Write;
        opts.open(&path)
            .and_then(|mut f| writeln!(f, "{service} {}", hash(&token)))
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(token)
    }

    /// The service a push token belongs to. The file is read on every push,
    /// so tokens added while the hub runs work at once.
    fn token_service(&self, token: &str) -> Option<String> {
        let h = hash(token);
        let text = std::fs::read_to_string(self.dir.join(TOKENS_FILE)).ok()?;
        text.lines().find_map(|l| {
            let (svc, th) = l.trim().split_once(' ')?;
            crate::http::same_secret(th.trim().as_bytes(), h.as_bytes()).then(|| svc.to_string())
        })
    }

    /// Apply a batch from a collector. Entries it already applied are
    /// skipped, so a retried batch is safe.
    pub fn apply(&self, batch: Batch) -> Result<Ack, String> {
        let store = self.store_or_create(&batch.service)?;
        let key = format!("push_seq:{}", batch.instance);
        let last: i64 = store
            .meta(&key)
            .map_err(db)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut acked = last;
        for entry in batch.entries {
            if entry.seq <= last {
                continue;
            }
            match entry.item {
                Item::Events { events } => {
                    let evs: Vec<_> = events.into_iter().map(|e| e.into_event()).collect();
                    store.record(&evs).map_err(db)?;
                }
                Item::Marker {
                    name,
                    at,
                    kind,
                    note,
                } => {
                    if kind == "deploy" {
                        store.add_deploy(&name, at, note.as_deref()).map_err(db)?;
                    } else {
                        store
                            .add_marker_once(&name, at, &kind, note.as_deref())
                            .map_err(db)?;
                    }
                }
                Item::Merge {
                    canonical,
                    template,
                    aliases,
                } => {
                    store
                        .merge_patterns(&crate::merge::MergeOp {
                            canonical,
                            template,
                            aliases,
                        })
                        .map_err(db)?;
                }
            }
            acked = acked.max(entry.seq);
        }
        if acked > last {
            store.set_meta(&key, Some(&acked.to_string())).map_err(db)?;
        }
        if let Some(env) = batch.env.as_deref().filter(|e| !e.is_empty())
            && store.meta("env").map_err(db)?.as_deref() != Some(env)
        {
            store.set_meta("env", Some(env)).map_err(db)?;
        }
        Ok(Ack { acked })
    }

    /// Every service's events of one request, in time order.
    pub fn request_lines(&self, request_id: &str) -> Result<Vec<ReqLine>, String> {
        let mut out = vec![];
        for (name, store) in self.stores() {
            let lines = store.request_lines(request_id, REQUEST_LINES).map_err(db)?;
            let levels = analysis::pattern_levels(&store, &lines)?;
            out.extend(lines.into_iter().map(|l| ReqLine {
                error: levels.get(&l.pattern_id).copied().unwrap_or(1) >= 3,
                service: Some(name.clone()),
                line: l,
            }));
        }
        out.sort_by(|a, b| a.line.ts.total_cmp(&b.line.ts));
        Ok(out)
    }

    /// Fold old counts into hourly ones in every service, like a collector does.
    pub fn spawn_pruner(self: &Arc<Self>, retention_secs: f64, history_secs: f64) {
        let hub = self.clone();
        std::thread::spawn(move || {
            loop {
                let now = analysis::now();
                for (_, s) in hub.stores() {
                    let _ = s.rollup(now - retention_secs, Some(now - history_secs));
                }
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        });
    }
}

// ---- push ----

/// `/hub/v1/push`, authenticated with collector tokens rather than the
/// hub's own token, so a collector can write its service but read nothing.
pub fn push_routes(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/hub/v1/push", axum::routing::post(push))
        .layer(DefaultBodyLimit::max(32 << 20))
        .with_state(hub)
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({"error": msg.into()}))).into_response()
}

async fn push(
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .unwrap_or("")
        .trim()
        .to_string();
    let h = hub.clone();
    let Ok(Some(service)) = tokio::task::spawn_blocking(move || h.token_service(&token)).await
    else {
        return err(
            StatusCode::UNAUTHORIZED,
            "Unknown push token. Create one with `starlist hub-token SERVICE`.",
        );
    };
    let batch: Batch = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("Bad batch: {e}")),
    };
    if batch.protocol != crate::push::PROTOCOL {
        return err(
            StatusCode::BAD_REQUEST,
            format!(
                "Push protocol {} isn't supported by this hub (it speaks {}); run the same starlist version on both.",
                batch.protocol,
                crate::push::PROTOCOL
            ),
        );
    }
    if batch.service != service {
        return err(
            StatusCode::FORBIDDEN,
            format!("This token pushes to '{service}', not '{}'.", batch.service),
        );
    }
    match tokio::task::spawn_blocking(move || hub.apply(batch)).await {
        Ok(Ok(ack)) => Json(ack).into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- API ----

/// `/api/v1` on a hub: every collector route, picked by `?service=`, plus
/// `/services` and answers across services.
pub fn api_routes(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/{*rest}", axum::routing::any(hub_api))
        .with_state(hub)
}

fn query_param(req: &Request, key: &str) -> Option<String> {
    let q = req.uri().query()?;
    url_pairs(q)
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

/// Decoded `key=value` pairs of a query string.
fn url_pairs(q: &str) -> impl Iterator<Item = (String, String)> + '_ {
    q.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decode(k), decode(v))
    })
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(x) => {
                    out.push(x);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Run a GET against one service's API and return its JSON.
async fn sub_get(hub: &Hub, service: &str, path_and_query: &str) -> Result<Value, String> {
    let api = hub
        .api(service)
        .ok_or_else(|| format!("Unknown service '{service}'."))?;
    let req = Request::builder()
        .uri(path_and_query)
        .body(Body::empty())
        .map_err(|e| e.to_string())?;
    let res = api.oneshot(req).await.map_err(|e| e.to_string())?;
    let ok = res.status().is_success();
    let bytes = axum::body::to_bytes(res.into_body(), 64 << 20)
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if ok {
        Ok(v)
    } else {
        Err(v["error"].as_str().unwrap_or("error").to_string())
    }
}

async fn hub_api(State(hub): State<Arc<Hub>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    if path == "/services" {
        return services(&hub).await;
    }
    if path == "/alerts/stream" {
        return err(
            StatusCode::NOT_FOUND,
            "Push alerts aren't available on a hub yet.",
        );
    }
    let names = hub.names();
    let service = match query_param(&req, "service") {
        Some(s) => s,
        None if path == "/status" && names.len() != 1 => return hub_status(&hub).await,
        None if path.starts_with("/requests/") => {
            let id = decode(path.trim_start_matches("/requests/"));
            return request_across(hub, id).await;
        }
        None if path == "/whats-new" && names.len() > 1 => {
            return whats_new_across(&hub, req.uri().query().unwrap_or("")).await;
        }
        None if names.len() == 1 => names[0].clone(),
        None => {
            let id = path
                .strip_prefix("/patterns/")
                .and_then(|r| r.split('/').next())
                .filter(|id| *id != "merge")
                .map(decode);
            let h = hub.clone();
            let found = match id {
                Some(id) => tokio::task::spawn_blocking(move || h.find_pattern(&id))
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            match found {
                Some(s) => s,
                None if names.is_empty() => {
                    return err(
                        StatusCode::NOT_FOUND,
                        "No collector has pushed to this hub yet.",
                    );
                }
                None => {
                    return err(
                        StatusCode::BAD_REQUEST,
                        format!("Pass service=NAME; this hub has {}.", names.join(", ")),
                    );
                }
            }
        }
    };
    // A fresh request, without this route's path parameters in its extensions.
    let (parts, body) = req.into_parts();
    let mut fresh = Request::new(body);
    *fresh.method_mut() = parts.method;
    *fresh.uri_mut() = parts.uri;
    *fresh.headers_mut() = parts.headers;
    match hub.api(&service) {
        Some(api) => match api.oneshot(fresh).await {
            Ok(r) => r.into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        },
        None => err(
            StatusCode::NOT_FOUND,
            format!("Unknown service '{service}'. GET /api/v1/services lists them."),
        ),
    }
}

async fn services(hub: &Hub) -> Response {
    let mut list = vec![];
    for name in hub.names() {
        let st = sub_get(hub, &name, "/status").await.unwrap_or(Value::Null);
        list.push(json!({
            "name": name,
            "env": st["env"],
            "current_release": st["current_release"],
            "patterns": st["patterns"],
            "last_hour": st["last_hour"],
        }));
    }
    Json(json!({"services": list})).into_response()
}

async fn hub_status(hub: &Hub) -> Response {
    let now = analysis::now();
    let (mut events, mut errors, mut patterns) = (0i64, 0i64, 0i64);
    let mut list = vec![];
    for name in hub.names() {
        let st = sub_get(hub, &name, "/status").await.unwrap_or(Value::Null);
        events += st["last_hour"]["events"].as_i64().unwrap_or(0);
        errors += st["last_hour"]["errors"].as_i64().unwrap_or(0);
        patterns += st["patterns"].as_i64().unwrap_or(0);
        list.push(json!({"name": name, "env": st["env"], "last_hour": st["last_hour"]}));
    }
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "now": crate::api::iso(now),
        "hub": true,
        "service": Value::Null,
        "patterns": patterns,
        "last_hour": {"events": events, "errors": errors},
        "services": list,
    }))
    .into_response()
}

async fn request_across(hub: Arc<Hub>, id: String) -> Response {
    let r = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let id = crate::request::normalize_query(&id);
        if id.is_empty() {
            return Err("Pass a request ID.".into());
        }
        let lines = hub.request_lines(&id)?;
        if lines.is_empty() {
            return Err(format!(
                "No events for request '{id}' in any service's raw buffer."
            ));
        }
        let start = lines[0].line.ts;
        let end = lines[lines.len() - 1].line.ts;
        let mut per: Vec<(String, usize, f64, f64)> = vec![];
        for l in &lines {
            let s = l.service.clone().unwrap_or_default();
            match per.iter_mut().find(|p| p.0 == s) {
                Some(p) => {
                    p.1 += 1;
                    p.3 = l.line.ts;
                }
                None => per.push((s, 1, l.line.ts, l.line.ts)),
            }
        }
        let ms = |t: f64| ((t - start) * 1000.0).round();
        Ok(json!({
            "request_id": id,
            "service": Value::Null,
            "started": crate::api::iso(start),
            "duration_ms": ms(end),
            "events": lines.len(),
            "truncated": false,
            "services": per.iter().map(|(s, n, a, b)| json!({
                "service": s, "events": n, "first_offset_ms": ms(*a), "last_offset_ms": ms(*b),
            })).collect::<Vec<_>>(),
            "lines": lines.iter().map(|l| json!({
                "id": l.line.id,
                "service": l.service,
                "at": crate::api::iso(l.line.ts),
                "offset_ms": ms(l.line.ts),
                "pattern_id": l.line.pattern_id,
                "text": crate::pattern::truncate(&l.line.text, 8000),
            })).collect::<Vec<_>>(),
        }))
    })
    .await;
    match r {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) if e.starts_with("No events") => err(StatusCode::NOT_FOUND, e),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `/whats-new` over every service: findings tagged with their service,
/// errors first.
async fn whats_new_across(hub: &Hub, query: &str) -> Response {
    let mut limit = 20usize;
    let mut offset = 0usize;
    let mut pass = vec![];
    for (k, v) in url_pairs(query) {
        match k.as_str() {
            "limit" => limit = v.parse().unwrap_or(20).clamp(1, 200),
            "offset" => offset = v.parse().unwrap_or(0),
            _ => pass.push(format!("{}={}", enc(&k), enc(&v))),
        }
    }
    pass.push("limit=200".into());
    let q = pass.join("&");
    let mut findings: Vec<Value> = vec![];
    let mut counts = BTreeMap::<String, i64>::new();
    let mut first: Option<Value> = None;
    for name in hub.names() {
        let v = match sub_get(hub, &name, &format!("/whats-new?{q}")).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_REQUEST, e),
        };
        if let Some(c) = v["counts"].as_object() {
            for (k, n) in c {
                *counts.entry(k.clone()).or_default() += n.as_i64().unwrap_or(0);
            }
        }
        for mut f in v["findings"].as_array().cloned().unwrap_or_default() {
            f["service"] = json!(name);
            findings.push(f);
        }
        first.get_or_insert(v);
    }
    let rank = |f: &Value| crate::pattern::level_rank(f["pattern"]["level"].as_str());
    findings.sort_by(|a, b| {
        rank(b).cmp(&rank(a)).then(
            b["count_window"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["count_window"].as_i64().unwrap_or(0)),
        )
    });
    let total = findings.len();
    let page: Vec<Value> = findings.into_iter().skip(offset).take(limit).collect();
    let base = first.unwrap_or(Value::Null);
    Json(json!({
        "window": base["window"],
        "baseline": base["baseline"],
        "service": Value::Null,
        "services": hub.names(),
        "counts": counts,
        "total": total,
        "offset": offset,
        "findings": page,
    }))
    .into_response()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ---- MCP ----

const INSTRUCTIONS: &str = "\
Distilled, redacted view of the logs of several services, pushed here by their collectors. Lines are grouped into patterns with stable IDs, per service.
Every tool takes `service` (list_services names them). whats_new without it covers every service; get_request without it follows one request across all of them.
Flow: whats_new -> get_pattern (get_context for surrounding lines, get_request for a request's path through the services) -> ship a fix -> verify_fix(pattern_id, since=marker, service).
Times: '30m', '2h', ISO time, or marker name. Output is brief by default; pass detail='full' for more.";

/// MCP on a hub: the collector tools with a `service` argument, plus
/// `list_services`.
#[derive(Clone)]
pub struct HubServer {
    hub: Arc<Hub>,
}

impl HubServer {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self { hub }
    }

    fn tools() -> Vec<Tool> {
        let mut tools: Vec<Tool> = LogServer::new_tools()
            .into_iter()
            .map(|mut t| {
                let mut schema = (*t.input_schema).clone();
                let props = schema
                    .entry("properties")
                    .or_insert_with(|| json!({}));
                if let Some(p) = props.as_object_mut() {
                    let desc = match t.name.as_ref() {
                        "whats_new" => "Service to ask, from list_services. Leave it out to cover every service.",
                        "get_request" => "Service to ask. Leave it out to follow the request across every service.",
                        _ => "Service to ask, from list_services. Can be left out when there is one service, or when a pattern_id says which.",
                    };
                    p.insert("service".into(), json!({"type": "string", "description": desc}));
                }
                t.input_schema = Arc::new(schema);
                t
            })
            .collect();
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), json!("object"));
        schema.insert("properties".into(), json!({}));
        tools.insert(
            0,
            Tool::new(
                "list_services",
                "Services that push to this hub, with their environment, live release and errors in the last hour.",
                Arc::new(schema),
            ),
        );
        tools
    }

    fn services_text(&self) -> String {
        let now = analysis::now();
        let list: Vec<Value> = self
            .hub
            .stores()
            .iter()
            .map(|(name, s)| {
                let errors = s
                    .level_series(now - 3600.0, now, 3600.0)
                    .unwrap_or_default()
                    .iter()
                    .filter(|(l, _)| crate::pattern::level_rank(Some(l)) >= 3)
                    .map(|(_, c)| c.iter().sum::<i64>())
                    .sum::<i64>();
                let release = s
                    .deploys()
                    .ok()
                    .and_then(|d| d.last().map(|(_, n)| n.clone()));
                json!({
                    "service": name,
                    "env": s.meta("env").ok().flatten(),
                    "release": release,
                    "patterns": s.pattern_count().ok(),
                    "errors_last_hour": errors,
                })
            })
            .collect();
        json!({"services": list}).to_string()
    }

    fn pick(&self, args: &serde_json::Map<String, Value>) -> Result<String, String> {
        let names = self.hub.names();
        if names.len() == 1 {
            return Ok(names[0].clone());
        }
        if let Some(id) = args.get("pattern_id").and_then(|v| v.as_str())
            && let Some(s) = self.hub.find_pattern(id)
        {
            return Ok(s);
        }
        if names.is_empty() {
            return Err("No collector has pushed to this hub yet.".into());
        }
        Err(format!("Pass service; this hub has {}.", names.join(", ")))
    }
}

fn text_result(r: Result<String, String>) -> CallToolResponse {
    match r {
        Ok(t) => CallToolResult::success(vec![ContentBlock::text(t)]).into(),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]).into(),
    }
}

/// The text of a tool result, and whether it was an error.
fn result_text(r: &CallToolResponse) -> Option<(String, bool)> {
    let CallToolResponse::Complete(r) = r else {
        return None;
    };
    let text = r.content.iter().find_map(|c| match c {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    })?;
    Some((text, r.is_error.unwrap_or(false)))
}

impl ServerHandler for HubServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "starlist-hub",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        Self::tools().into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let mut args = request.arguments.take().unwrap_or_default();
        let service = args
            .remove("service")
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.trim().is_empty());
        let name = request.name.to_string();
        if name == "list_services" {
            return Ok(text_result(Ok(self.services_text())));
        }
        if service.is_none() && name == "get_request" {
            let hub = self.hub.clone();
            let r = tokio::task::spawn_blocking(move || -> Result<String, String> {
                let id = crate::request::normalize_query(
                    args.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                );
                if id.is_empty() {
                    return Err("Pass a request_id, e.g. the example_request of a finding.".into());
                }
                let d = Detail::parse(args.get("detail").and_then(|v| v.as_str()))?;
                let max = args
                    .get("max_chars")
                    .and_then(|v| v.as_u64())
                    .map(|m| m as usize)
                    .unwrap_or_else(|| analysis::Page::default_budget(d))
                    .max(analysis::MIN_MAX_CHARS);
                let lines = hub.request_lines(&id)?;
                if lines.is_empty() {
                    return Err(format!(
                        "No events for request '{id}' in any service. Collectors send raw lines unless they run with --no-raw."
                    ));
                }
                let v = analysis::request_timeline(&id, lines, max, d, analysis::now())?;
                Ok(serde_json::to_string(&analysis::compact(v)).unwrap_or_default())
            })
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            return Ok(text_result(r));
        }
        if service.is_none() && name == "whats_new" && self.hub.names().len() > 1 {
            // Each service answers within its share of the budget.
            let names = self.hub.names();
            let full = args.get("detail").and_then(|v| v.as_str()) == Some("full");
            let budget = args
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(if full { 16000 } else { 8000 }) as usize;
            let share = (budget / names.len()).max(analysis::MIN_MAX_CHARS);
            let mut out = serde_json::Map::new();
            for svc in names {
                let mut a = args.clone();
                a.insert("max_chars".into(), json!(share));
                a.remove("cursor");
                let mut req = request.clone();
                req.arguments = Some(a);
                let store = self.hub.store(&svc).expect("listed service");
                let r = LogServer::new(store)
                    .call_tool(req, context.clone())
                    .await?;
                let v = match result_text(&r) {
                    Some((t, false)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
                    Some((t, true)) => json!({"error": t}),
                    None => Value::Null,
                };
                out.insert(svc, v);
            }
            return Ok(text_result(Ok(json!({"by_service": out}).to_string())));
        }
        let svc = match service {
            Some(s) => s,
            None => match self.pick(&args) {
                Ok(s) => s,
                Err(e) => return Ok(text_result(Err(e))),
            },
        };
        let Some(store) = self.hub.store(&svc) else {
            return Ok(text_result(Err(format!(
                "Unknown service '{svc}'. list_services names them."
            ))));
        };
        request.arguments = Some(args);
        LogServer::new(store).call_tool(request, context).await
    }
}
