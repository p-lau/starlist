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
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn require_bearer(State(token): State<Arc<String>>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .is_some_and(|t| same(t.trim().as_bytes(), token.as_bytes()));
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
        },
    )
}

/// The HTTP server: `/mcp` and, unless turned off, `/api/v1` and the dashboard at `/`.
///
/// - With a token, every request needs `Authorization: Bearer <token>`. The Host
///   check is then only applied if `allowed_hosts` is given, since a browser on a
///   rebinding attack can't supply the token anyway.
/// - Without a token, only loopback Host headers are accepted (rmcp's default),
///   which protects a local server from DNS rebinding.
/// - API writes need a same-origin or allowed `Origin`, so another site can't
///   make a visitor's browser change anything.
pub fn app(store: Arc<Store>, cfg: HttpConfig) -> axum::Router {
    let mut config = StreamableHttpServerConfig::default();
    if !cfg.allowed_hosts.is_empty() {
        config = config.with_allowed_hosts(cfg.allowed_hosts.clone());
    } else if cfg.token.is_some() {
        config = config.disable_allowed_hosts();
    }
    let alerts = cfg.alerts.clone();
    let mcp_store = store.clone();
    let service: StreamableHttpService<LogServer, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(LogServer::new(mcp_store.clone()).with_alerts(alerts.clone())),
        Default::default(),
        config,
    );
    let token = cfg.token.clone().map(Arc::new);
    let mut mcp = axum::Router::new().nest_service("/mcp", service);
    if let Some(t) = &token {
        mcp = mcp.layer(middleware::from_fn_with_state(t.clone(), require_bearer));
    }
    if !cfg.api {
        return mcp;
    }
    let mut api = crate::api::routes(crate::api::ApiState {
        store,
        alerts: cfg.alerts.clone(),
    });
    if let Some(t) = &token {
        api = api.layer(middleware::from_fn_with_state(t.clone(), require_bearer));
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
    let api = api.layer(middleware::from_fn_with_state(guard, guard_api));
    let router = mcp.nest("/api/v1", api);
    if cfg.dashboard {
        router.fallback(crate::dashboard::serve)
    } else {
        router
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
