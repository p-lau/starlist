//! MCP and the JSON API over HTTP, with optional bearer-token auth.

use crate::server::LogServer;
use crate::store::Store;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::Arc;

/// Compare without leaking how many leading bytes matched.
pub fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) async fn require_bearer(
    State(token): State<Arc<String>>,
    req: Request,
    next: Next,
) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .is_some_and(|t| same_secret(t.trim().as_bytes(), token.as_bytes()));
    if ok {
        return next.run(req).await;
    }
    let mut res = (StatusCode::UNAUTHORIZED, "missing or wrong bearer token\n").into_response();
    res.headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    res
}

/// What the HTTP server serves and how it guards it.
#[derive(Clone, Default)]
pub struct HttpConfig {
    /// Require `Authorization: Bearer <token>` on `/mcp` and `/api`.
    pub token: Option<String>,
    /// Host names clients may use; empty means loopback only without a token,
    /// and any host with one.
    pub allowed_hosts: Vec<String>,
    pub alerts: Option<Arc<crate::alerts::Alerts>>,
    /// Serve the JSON API under `/api/v1`.
    pub api: bool,
    /// Origins allowed to call the API from a browser (CORS).
    pub cors_origins: Vec<String>,
    /// Serve the dashboard at `/` (needs the API).
    pub dashboard: bool,
    /// Serve Prometheus metrics at `/metrics`.
    pub prometheus: bool,
    /// Offer only these MCP tools; `None` offers them all.
    pub tools: Option<Vec<String>>,
}

/// The `/mcp` endpoint only (and the API, which is on by default).
pub fn router(
    store: Arc<Store>,
    token: Option<String>,
    allowed_hosts: Vec<String>,
    alerts: Option<Arc<crate::alerts::Alerts>>,
) -> axum::Router {
    app(
        store,
        HttpConfig {
            token,
            allowed_hosts,
            alerts,
            api: true,
            cors_origins: vec![],
            dashboard: false,
            prometheus: false,
            tools: None,
        },
    )
}

/// The HTTP server: `/mcp` and, unless turned off, `/api/v1` and the dashboard at
/// `/`, plus `/metrics` for Prometheus when asked for.
///
/// - With a token, every request needs `Authorization: Bearer <token>`. The Host
///   check is then only applied if `allowed_hosts` is given, since a browser on a
///   rebinding attack can't supply the token anyway.
/// - Without a token, only loopback Host headers are accepted (rmcp's default),
///   which protects a local server from DNS rebinding.
/// - API writes need a same-origin or allowed `Origin`, so another site can't
///   make a visitor's browser change anything.
pub fn app(store: Arc<Store>, cfg: HttpConfig) -> axum::Router {
    let alerts = cfg.alerts.clone();
    let mcp_store = store.clone();
    let tools = cfg.tools.clone();
    let mcp = mcp_service(&cfg, move || {
        Ok(LogServer::new(mcp_store.clone())
            .with_alerts(alerts.clone())
            .with_tools(tools.as_deref()))
    });
    let mut guarded = axum::Router::new();
    if cfg.api {
        guarded = guarded.nest(
            "/api/v1",
            crate::api::routes(crate::api::ApiState {
                store: store.clone(),
                alerts: cfg.alerts.clone(),
                service: None,
            }),
        );
    }
    #[cfg(feature = "prometheus")]
    if cfg.prometheus {
        guarded = guarded.route(
            "/metrics",
            axum::routing::get(move || prometheus(store.clone())),
        );
    }
    assemble(mcp, guarded, &cfg)
}

#[cfg(feature = "hub")]
/// A hub: the same MCP tools, API and dashboard over every service that
/// pushes to it, plus `/hub/v1/push` for collectors (with their own tokens).
pub fn hub_app(hub: Arc<crate::hub::Hub>, cfg: HttpConfig) -> axum::Router {
    let mcp_hub = hub.clone();
    let mcp = mcp_service(&cfg, move || {
        Ok(crate::hub::HubServer::new(mcp_hub.clone()))
    });
    let mut guarded = axum::Router::new();
    if cfg.api {
        guarded = guarded.nest("/api/v1", crate::hub::api_routes(hub.clone()));
    }
    assemble(mcp, guarded, &cfg).merge(crate::hub::push_routes(hub))
}

/// `/mcp`, behind the token when there is one.
fn mcp_service<S, F>(cfg: &HttpConfig, factory: F) -> axum::Router
where
    S: rmcp::ServerHandler + Send + 'static,
    F: Fn() -> Result<S, std::io::Error> + Send + Sync + 'static,
{
    let mut config = StreamableHttpServerConfig::default();
    if !cfg.allowed_hosts.is_empty() {
        config = config.with_allowed_hosts(cfg.allowed_hosts.clone());
    } else if cfg.token.is_some() {
        config = config.disable_allowed_hosts();
    }
    let service: StreamableHttpService<S, LocalSessionManager> =
        StreamableHttpService::new(factory, Default::default(), config);
    let mut mcp = axum::Router::new().nest_service("/mcp", service);
    if let Some(t) = &cfg.token {
        mcp = mcp.layer(middleware::from_fn_with_state(
            Arc::new(t.clone()),
            require_bearer,
        ));
    }
    mcp
}

/// Put the API (and `/metrics`) behind the token, Host and Origin checks,
/// next to `/mcp`, with the dashboard as the fallback.
fn assemble(mcp: axum::Router, mut guarded: axum::Router, cfg: &HttpConfig) -> axum::Router {
    if !cfg.api && !cfg.prometheus {
        return mcp;
    }
    if let Some(t) = &cfg.token {
        guarded = guarded.layer(middleware::from_fn_with_state(
            Arc::new(t.clone()),
            require_bearer,
        ));
    }
    let guard = Arc::new(Guard {
        token: cfg.token.is_some(),
        allowed_hosts: cfg.allowed_hosts.clone(),
        cors_origins: cfg
            .cors_origins
            .iter()
            .map(|o| o.trim().trim_end_matches('/').to_ascii_lowercase())
            .collect(),
    });
    // The guard is outside the token check so CORS preflights, which carry no
    // credentials, get an answer.
    let router = mcp.merge(guarded.layer(middleware::from_fn_with_state(guard, guard_api)));
    #[cfg(feature = "dashboard")]
    if cfg.api && cfg.dashboard {
        return router.fallback(crate::dashboard::serve);
    }
    router
}

#[cfg(feature = "prometheus")]
async fn prometheus(store: Arc<Store>) -> Response {
    let now = crate::analysis::now();
    match tokio::task::spawn_blocking(move || crate::prometheus::render(&store, now)).await {
        Ok(Ok(text)) => (
            [(
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            text,
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

struct Guard {
    token: bool,
    allowed_hosts: Vec<String>,
    cors_origins: Vec<String>,
}

/// "host:port" or "[v6]:port" to the host alone.
fn host_only(h: &str) -> &str {
    if let Some(rest) = h.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match h.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            host
        }
        _ => h,
    }
}

fn is_loopback_host(h: &str) -> bool {
    let h = host_only(h);
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn json_error(code: StatusCode, msg: &str) -> Response {
    (code, axum::Json(serde_json::json!({"error": msg}))).into_response()
}

async fn guard_api(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let host_ok = if !g.allowed_hosts.is_empty() {
        g.allowed_hosts
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&host) || a.eq_ignore_ascii_case(host_only(&host)))
    } else {
        g.token || is_loopback_host(&host)
    };
    if !host_ok {
        return json_error(StatusCode::FORBIDDEN, "Host not allowed.");
    }
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|o| o.trim_end_matches('/').to_ascii_lowercase());
    let cors = origin
        .as_ref()
        .filter(|o| g.cors_origins.iter().any(|c| c == *o))
        .cloned();
    if let Some(o) = &cors
        && req.method() == Method::OPTIONS
    {
        let mut res = StatusCode::NO_CONTENT.into_response();
        let h = res.headers_mut();
        cors_headers(h, o);
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        return res;
    }
    let reads = matches!(*req.method(), Method::GET | Method::HEAD);
    if !reads && cors.is_none() {
        let same = origin.as_ref().is_none_or(|o| {
            o.split_once("://")
                .is_some_and(|(_, authority)| authority.eq_ignore_ascii_case(&host))
        });
        if !same {
            return json_error(
                StatusCode::FORBIDDEN,
                "Cross-origin writes are refused; allow the origin with --cors-origin.",
            );
        }
    }
    let mut res = next.run(req).await;
    if let Some(o) = &cors {
        cors_headers(res.headers_mut(), o);
    }
    res
}

fn cors_headers(h: &mut axum::http::HeaderMap, origin: &str) {
    if let Ok(v) = HeaderValue::from_str(origin) {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    h.append(header::VARY, HeaderValue::from_static("Origin"));
}

/// A new random token: 32 bytes, hex encoded.
pub fn generate_token() -> std::io::Result<String> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(std::io::Error::other)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
