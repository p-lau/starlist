//! MCP over HTTP, with optional bearer-token auth.

use crate::server::LogServer;
use crate::store::Store;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
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

/// The `/mcp` endpoint.
///
/// - With a token, every request needs `Authorization: Bearer <token>`. The Host
///   check is then only applied if `allowed_hosts` is given, since a browser on a
///   rebinding attack can't supply the token anyway.
/// - Without a token, only loopback Host headers are accepted (rmcp's default),
///   which protects a local server from DNS rebinding.
pub fn router(
    store: Arc<Store>,
    token: Option<String>,
    allowed_hosts: Vec<String>,
    alerts: Option<Arc<crate::alerts::Alerts>>,
) -> axum::Router {
    let mut config = StreamableHttpServerConfig::default();
    if !allowed_hosts.is_empty() {
        config = config.with_allowed_hosts(allowed_hosts);
    } else if token.is_some() {
        config = config.disable_allowed_hosts();
    }
    let service: StreamableHttpService<LogServer, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(LogServer::new(store.clone()).with_alerts(alerts.clone())),
        Default::default(),
        config,
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    match token {
        Some(t) => router.layer(middleware::from_fn_with_state(Arc::new(t), require_bearer)),
        None => router,
    }
}

/// A new random token: 32 bytes, hex encoded.
pub fn generate_token() -> std::io::Result<String> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(std::io::Error::other)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
