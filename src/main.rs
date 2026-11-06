use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use regex::Regex;
use rmcp::ServiceExt;
use starlist::collector::{self, DEFAULT_MARKER_REGEX, Pipeline};
use starlist::redact::Redactor;
use starlist::server::LogServer;
use starlist::store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Parser)]
#[command(
    name = "starlist",
    version,
    about = "Watches your app's logs and serves distilled, redacted summaries to AI agents over MCP."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Collect logs and serve MCP from the same process (the usual way to run it).
    Serve {
        #[command(flatten)]
        src: SourceArgs,
        #[command(flatten)]
        mcp: McpArgs,
        #[command(flatten)]
        alerts: AlertArgs,
    },
    /// Collect logs only; run `starlist mcp` separately against the same --db.
    Collect {
        #[command(flatten)]
        src: SourceArgs,
        #[command(flatten)]
        alerts: AlertArgs,
    },
    /// Serve MCP from an existing database without collecting.
    Mcp {
        #[arg(long, default_value = "starlist.db")]
        db: PathBuf,
        #[command(flatten)]
        mcp: McpArgs,
        #[command(flatten)]
        alerts: AlertArgs,
    },
    /// Import existing log files from the beginning, then exit.
    Ingest {
        files: Vec<PathBuf>,
        #[command(flatten)]
        opts: PipelineArgs,
    },
    /// Record a named marker (for example a deploy) at the current time.
    Mark {
        name: String,
        #[arg(long)]
        note: Option<String>,
        /// When it happened, if not now: ISO time or duration back from now ("5m").
        #[arg(long)]
        at: Option<String>,
        /// Mark a release going live; patterns first seen after it are attributed to it.
        #[arg(long)]
        deploy: bool,
        #[arg(long, default_value = "starlist.db")]
        db: PathBuf,
    },
}

#[derive(Args, Clone)]
struct AlertArgs {
    /// POST a JSON alert here when a new or spiking ERROR/FATAL pattern appears
    /// (for setups without a connected agent).
    #[arg(long, env = "STARLIST_NOTIFY_URL")]
    notify_url: Option<String>,
    /// Turn off alerts: no resource subscriptions and no webhook.
    #[arg(long)]
    no_alerts: bool,
}

impl AlertArgs {
    /// Start the alert watcher if anything can receive alerts.
    fn start(
        &self,
        store: &Arc<Store>,
        serving_mcp: bool,
    ) -> Option<Arc<starlist::alerts::Alerts>> {
        if self.no_alerts || (!serving_mcp && self.notify_url.is_none()) {
            return None;
        }
        let cfg = starlist::alerts::AlertConfig {
            notify_url: self.notify_url.clone(),
            ..Default::default()
        };
        Some(starlist::alerts::spawn_watcher(store.clone(), cfg))
    }
}

#[derive(Args, Clone)]
struct PipelineArgs {
    /// SQLite file for patterns and counts.
    #[arg(long, default_value = "starlist.db")]
    db: PathBuf,
    /// Extra regex to redact (repeatable).
    #[arg(long = "redact")]
    redact: Vec<String>,
    /// Keep IPv4 addresses instead of redacting them.
    #[arg(long)]
    keep_ips: bool,
    /// Regex with a (?P<name>...) group that marks a deploy when it matches a log line.
    #[arg(long, default_value = DEFAULT_MARKER_REGEX)]
    marker_regex: String,
    /// Turn off deploy detection from log lines.
    #[arg(long)]
    no_auto_markers: bool,
    /// UTC offset for log timestamps that have no zone, like "+02:00" or "-0800". Default UTC.
    #[arg(long, value_parser = parse_tz)]
    log_tz: Option<i32>,
    /// Path prefix to strip from code locations so they match your repository,
    /// like "/app" in a container (repeatable).
    #[arg(long = "source-root")]
    source_roots: Vec<String>,
    /// Disk space for recent redacted lines that get_context reads, like "20MB" or "1GB".
    #[arg(long, default_value = "20MB", value_parser = parse_size)]
    raw_size: i64,
    /// Extra field that holds a request ID, beyond trace_id, request_id,
    /// traceparent and similar (repeatable, any spelling).
    #[arg(long = "request-key")]
    request_keys: Vec<String>,
    /// Keep no raw lines (get_context and get_request are then unavailable) and delete any already stored.
    #[arg(long)]
    no_raw: bool,
    /// Merge patterns that differ in one word once it has taken this many values (0 turns merging off).
    #[arg(long, default_value_t = starlist::merge::DEFAULT_THRESHOLD)]
    merge_after: usize,
}

#[derive(Args, Clone)]
struct SourceArgs {
    /// Log file to follow (repeatable). Survives rotation.
    #[arg(long = "file")]
    files: Vec<PathBuf>,
    /// Read logs from stdin, e.g. `npm start | starlist serve --stdin --tee`.
    #[arg(long)]
    stdin: bool,
    /// With --stdin, also print the logs to stdout.
    #[arg(long)]
    tee: bool,
    /// Read files from the beginning instead of only new lines.
    #[arg(long)]
    from_start: bool,
    /// How long to keep counts, like "7d".
    #[arg(long, default_value = "7d")]
    retention: String,
    /// Version of the app being watched, recorded as a deploy marker at startup
    /// (when it differs from the release already live).
    #[arg(long, env = "STARLIST_VERSION")]
    app_version: Option<String>,
    #[command(flatten)]
    opts: PipelineArgs,
}

#[derive(Args, Clone)]
struct McpArgs {
    /// Serve MCP over stdio instead of HTTP (the agent launches starlist itself).
    #[arg(long)]
    stdio: bool,
    /// Address for MCP over HTTP; the endpoint is http://ADDR/mcp.
    #[arg(long, default_value = "127.0.0.1:7341")]
    http: String,
    /// Require `Authorization: Bearer TOKEN` on HTTP. When listening beyond localhost
    /// without one, a token is generated and saved next to the database (`<db>.token`).
    #[arg(long, env = "STARLIST_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Host names clients use to reach this server (repeatable). Only needed to keep
    /// Host-header checks on when listening beyond localhost.
    #[arg(long = "allowed-host")]
    allowed_hosts: Vec<String>,
    /// Don't serve the JSON API at /api/v1 (MCP only).
    #[arg(long)]
    no_api: bool,
    /// Let a web UI on this origin call the API from a browser, like
    /// "http://localhost:5173" (repeatable). Off by default.
    #[arg(long = "cors-origin")]
    cors_origins: Vec<String>,
}

fn parse_size(s: &str) -> Result<i64, String> {
    let t = s.trim().to_ascii_uppercase();
    let (num, mult) = [
        ("GB", 1i64 << 30),
        ("MB", 1 << 20),
        ("KB", 1 << 10),
        ("B", 1),
    ]
    .iter()
    .find_map(|(u, m)| t.strip_suffix(u).map(|n| (n.trim().to_string(), *m)))
    .unwrap_or((t.clone(), 1));
    num.parse::<f64>()
        .ok()
        .filter(|n| *n >= 0.0)
        .map(|n| (n * mult as f64) as i64)
        .ok_or_else(|| format!("'{s}' is not a size; use something like 20MB"))
}

fn parse_tz(s: &str) -> Result<i32, String> {
    starlist::pattern::parse_utc_offset(s)
        .ok_or_else(|| format!("'{s}' is not a UTC offset; use e.g. UTC, +02:00 or -0800"))
}

fn is_loopback(addr: &str) -> bool {
    match addr.parse::<std::net::SocketAddr>() {
        Ok(a) => a.ip().is_loopback(),
        Err(_) => addr
            .split(':')
            .next()
            .is_some_and(|h| h.eq_ignore_ascii_case("localhost")),
    }
}

/// The token to require, if any. Listening beyond localhost always needs one.
fn resolve_token(mcp: &McpArgs, db: &Path) -> Result<Option<String>> {
    if let Some(t) = &mcp.token {
        if t.trim().len() < 16 {
            bail!("--token should be at least 16 characters");
        }
        return Ok(Some(t.trim().to_string()));
    }
    if is_loopback(&mcp.http) {
        return Ok(None);
    }
    let path = db.with_extension("token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        eprintln!("starlist: requiring the bearer token in {}", path.display());
        return Ok(Some(t.trim().to_string()));
    }
    let t = starlist::http::generate_token()?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    use std::io::Write;
    opts.open(&path)
        .and_then(|mut f| f.write_all(t.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!(
        "starlist: listening beyond localhost, so a bearer token is required. Generated one in {}",
        path.display()
    );
    Ok(Some(t))
}

fn open_store(db: &PathBuf) -> Result<Arc<Store>> {
    Ok(Arc::new(
        Store::open(db).with_context(|| format!("opening {}", db.display()))?,
    ))
}

fn pipeline(opts: &PipelineArgs, store: Arc<Store>) -> Result<Pipeline> {
    let extra = opts
        .redact
        .iter()
        .map(|r| Regex::new(r))
        .collect::<Result<Vec<_>, _>>()
        .context("bad --redact regex")?;
    let marker = if opts.no_auto_markers {
        None
    } else {
        Some(Regex::new(&opts.marker_regex).context("bad --marker-regex")?)
    };
    let merge = (opts.merge_after > 0).then_some(opts.merge_after);
    store.set_raw_cap((!opts.no_raw).then_some(opts.raw_size))?;
    Ok(
        Pipeline::with_merging(store, Redactor::new(!opts.keep_ips, extra), marker, merge)
            .with_log_tz(opts.log_tz.unwrap_or(0))
            .with_source_roots(opts.source_roots.clone())
            .with_request_keys(&opts.request_keys),
    )
}

/// Start collection threads. Returns a handle that finishes when stdin closes (if reading stdin).
fn start_collection(
    src: &SourceArgs,
    store: Arc<Store>,
) -> Result<Option<std::thread::JoinHandle<()>>> {
    if src.files.is_empty() && !src.stdin {
        bail!("nothing to collect: pass --file PATH and/or --stdin");
    }
    let retention =
        starlist::analysis::parse_duration(&src.retention).context("bad --retention")?;
    collector::spawn_pruner(store.clone(), retention);
    if let Some(v) = src
        .app_version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        && store.add_deploy(v, starlist::analysis::now(), None)?
    {
        eprintln!("starlist: deploy marker {v}");
    }
    let p = Arc::new(pipeline(&src.opts, store)?);
    static STOP: AtomicBool = AtomicBool::new(false);
    for f in &src.files {
        let (p, f, from_start) = (p.clone(), f.clone(), src.from_start);
        eprintln!("starlist: following {}", f.display());
        std::thread::spawn(move || collector::tail_file(&p, f, from_start, false, &STOP));
    }
    if src.stdin {
        let (p, tee) = (p.clone(), src.tee);
        return Ok(Some(std::thread::spawn(move || {
            collector::read_stdin(&p, tee)
        })));
    }
    Ok(None)
}

async fn run_mcp(
    store: Arc<Store>,
    mcp: &McpArgs,
    db: &Path,
    alerts: Option<Arc<starlist::alerts::Alerts>>,
) -> Result<()> {
    if mcp.stdio {
        eprintln!("starlist: serving MCP over stdio");
        let svc = LogServer::new(store)
            .with_alerts(alerts)
            .serve(rmcp::transport::stdio())
            .await?;
        svc.waiting().await?;
        return Ok(());
    }
    let token = resolve_token(mcp, db)?;
    let api = !mcp.no_api;
    let router = starlist::http::app(
        store,
        starlist::http::HttpConfig {
            token,
            allowed_hosts: mcp.allowed_hosts.clone(),
            alerts,
            api,
            cors_origins: mcp.cors_origins.clone(),
        },
    );
    let listener = tokio::net::TcpListener::bind(&mcp.http)
        .await
        .with_context(|| format!("binding {}", mcp.http))?;
    let addr = listener.local_addr()?;
    eprintln!("starlist: MCP endpoint http://{addr}/mcp");
    if api {
        eprintln!("starlist: JSON API http://{addr}/api/v1");
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve { src, mcp, alerts } => {
            if mcp.stdio && src.stdin {
                bail!(
                    "--stdin can't be combined with --stdio (both use standard input); use HTTP or --file"
                );
            }
            let store = open_store(&src.opts.db)?;
            start_collection(&src, store.clone())?;
            let alerts = alerts.start(&store, true);
            run_mcp(store, &mcp, &src.opts.db, alerts).await
        }
        Cmd::Collect { src, alerts } => {
            let store = open_store(&src.opts.db)?;
            let _alerts = alerts.start(&store, false);
            match start_collection(&src, store)? {
                Some(h) => {
                    let _ = tokio::task::spawn_blocking(move || h.join()).await;
                }
                None => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
            Ok(())
        }
        Cmd::Mcp { db, mcp, alerts } => {
            let store = open_store(&db)?;
            let alerts = alerts.start(&store, true);
            run_mcp(store, &mcp, &db, alerts).await
        }
        Cmd::Ingest { files, opts } => {
            let store = open_store(&opts.db)?;
            let p = pipeline(&opts, store)?;
            let stop = AtomicBool::new(false);
            for f in files {
                collector::tail_file(&p, f.clone(), true, true, &stop);
                eprintln!("starlist: imported {}", f.display());
            }
            Ok(())
        }
        Cmd::Mark {
            name,
            note,
            at,
            deploy,
            db,
        } => {
            let store = open_store(&db)?;
            let v = starlist::analysis::add_marker(
                &store,
                &name,
                note.as_deref(),
                at.as_deref(),
                deploy,
                starlist::analysis::now(),
            )
            .map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(())
        }
    }
}
