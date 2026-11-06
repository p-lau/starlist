//! The dashboard is served at / without shadowing /mcp or the API (issue #22).

use starlist::http::{HttpConfig, app};
use starlist::store::Store;
use std::sync::Arc;

async fn serve(dashboard: bool) -> String {
    let db = std::env::temp_dir().join(format!(
        "starlist-dash-{}-{dashboard}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db);
    let router = app(
        Arc::new(Store::open(&db).unwrap()),
        HttpConfig {
            api: true,
            dashboard,
            ..Default::default()
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{addr}")
}

#[tokio::test]
async fn serves_the_app_shell_for_pages() {
    let base = serve(true).await;
    for path in ["/", "/patterns", "/patterns/p123", "/deploys"] {
        let r = reqwest::get(format!("{base}{path}")).await.unwrap();
        assert_eq!(r.status().as_u16(), 200, "{path}");
        assert!(
            r.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "{path}"
        );
        let csp = r.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("connect-src 'self'"));
    }
    // The API and MCP are not shadowed.
    let r = reqwest::get(format!("{base}/api/v1/nothing"))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    assert!(r.text().await.unwrap().contains("error"));
    let r = reqwest::get(format!("{base}/api/v1/status")).await.unwrap();
    assert_eq!(r.status().as_u16(), 200);

    if starlist::dashboard::embedded() {
        let r = reqwest::get(format!("{base}/_app/missing.js"))
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 404);
        let r = reqwest::get(format!("{base}/favicon.svg")).await.unwrap();
        assert_eq!(r.headers()["content-type"], "image/svg+xml");
    }
}

#[tokio::test]
async fn can_be_turned_off() {
    let base = serve(false).await;
    let r = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(r.status().as_u16(), 404);
}
