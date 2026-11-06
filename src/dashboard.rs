//! The dashboard: a static Svelte app embedded at build time and served at `/`.

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

mod files {
    include!(concat!(env!("OUT_DIR"), "/dashboard.rs"));
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>starlist</title>
<style>body{font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:4rem auto;padding:0 1rem}code{background:#8882;padding:2px 5px;border-radius:4px}</style>
</head><body>
<h1>starlist</h1>
<p>This build of starlist doesn't include the dashboard. Build it, then build starlist again:</p>
<p><code>cd dashboard &amp;&amp; npm ci &amp;&amp; npm run build</code><br><code>cargo build --release</code></p>
<p>MCP is at <code>/mcp</code> and the JSON API at <code>/api/v1</code>.</p>
</body></html>"#;

/// The dashboard only talks to its own server and loads nothing from elsewhere.
const CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'";

fn secure(h: &mut axum::http::HeaderMap) {
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
}

/// Whether the dashboard was built into this binary.
pub fn embedded() -> bool {
    !files::FILES.is_empty()
}

fn find(path: &str) -> Option<&'static files::File> {
    files::FILES.iter().find(|(p, _, _)| *p == path)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serve a dashboard file, or the app shell for any other page path.
pub async fn serve(req: Request) -> Response {
    if !embedded() {
        let mut res = Html(PLACEHOLDER).into_response();
        secure(res.headers_mut());
        return res;
    }
    let path = req.uri().path().trim_start_matches('/');
    let file = match find(path) {
        Some(f) => f,
        // A file-like path that doesn't exist is a 404; anything else is a page
        // of the single-page app.
        None if path.rsplit('/').next().unwrap_or("").contains('.') => {
            return StatusCode::NOT_FOUND.into_response();
        }
        None => match find("200.html") {
            Some(f) => f,
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let gzip_ok = req
        .headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|e| e.trim().starts_with("gzip")));
    let (name, raw, gz) = *file;
    let mut res = match (gz, gzip_ok) {
        (Some(gz), true) => {
            let mut r = gz.into_response();
            r.headers_mut()
                .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            r
        }
        _ => raw.into_response(),
    };
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(name)),
    );
    h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if name.starts_with("_app/immutable/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    secure(h);
    res
}
