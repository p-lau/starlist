use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use starlist::collector::{self, DEFAULT_MARKER_REGEX, Pipeline};
use starlist::redact::Redactor;
use starlist::server::LogServer;
use starlist::store::Store;
use regex::Regex;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Parser)]
#[command(name = "starlist", version, about = "Watches your app's logs and serves distilled, redacted summaries to AI agents over MCP.")]
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
    },
    /// Collect logs only; run `starlist mcp` separately against the same --db.
    Collect {
        #[command(flatten)]
        src: SourceArgs,
    },
    /// Serve MCP from an existing database without collecting.
    Mcp {
        #[arg(long, default_value = "starlist.db")]
        db: PathBuf,
        #[command(flatten)]
        mcp: McpArgs,
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
        #[arg(long, default_value = "starlist.db")]
        db: PathBuf,
    },
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
}

fn open_store(db: &PathBuf) -> Result<Arc<Store>> {
    Ok(Arc::new(Store::open(db).with_context(|| format!("opening {}", db.display()))?))
}

fn pipeline(opts: &PipelineArgs, store: Arc<Store>) -> Result<Pipeline> {
    let extra = opts.redact.iter().map(|r| Regex::new(r)).collect::<Result<Vec<_>, _>>().context("bad --redact regex")?;
    let marker = if opts.no_auto_markers {
        None
    } else {
        Some(Regex::new(&opts.marker_regex).context("bad --marker-regex")?)
    };
    Ok(Pipeline::new(store, Redactor::new(!opts.keep_ips, extra), marker))
}

/// Start collection threads. Returns a handle that finishes when stdin closes (if reading stdin).
fn start_collection(src: &SourceArgs, store: Arc<Store>) -> Result<Option<std::thread::JoinHandle<()>>> {
    if src.files.is_empty() && !src.stdin {
        bail!("nothing to collect: pass --file PATH and/or --stdin");
    }
    let retention = starlist::analysis::parse_duration(&src.retention).context("bad --retention")?;
    collector::spawn_pruner(store.clone(), retention);
    let p = Arc::new(pipeline(&src.opts, store)?);
    static STOP: AtomicBool = AtomicBool::new(false);
    for f in &src.files {
        let (p, f, from_start) = (p.clone(), f.clone(), src.from_start);
        eprintln!("starlist: following {}", f.display());
        std::thread::spawn(move || collector::tail_file(&p, f, from_start, false, &STOP));
    }
    if src.stdin {
        let (p, tee) = (p.clone(), src.tee);
        return Ok(Some(std::thread::spawn(move || collector::read_stdin(&p, tee))));
    }
    Ok(None)
}

async fn run_mcp(store: Arc<Store>, mcp: &McpArgs) -> Result<()> {
    if mcp.stdio {
        eprintln!("starlist: serving MCP over stdio");
        let svc = LogServer::new(store).serve(rmcp::transport::stdio()).await?;
        svc.waiting().await?;
        return Ok(());
    }
    let service: StreamableHttpService<LogServer, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(LogServer::new(store.clone())),
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&mcp.http).await.with_context(|| format!("binding {}", mcp.http))?;
    eprintln!("starlist: MCP endpoint http://{}/mcp", listener.local_addr()?);
    axum::serve(listener, router).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve { src, mcp } => {
            if mcp.stdio && src.stdin {
                bail!("--stdin can't be combined with --stdio (both use standard input); use HTTP or --file");
            }
            let store = open_store(&src.opts.db)?;
            start_collection(&src, store.clone())?;
            run_mcp(store, &mcp).await
        }
        Cmd::Collect { src } => {
            let store = open_store(&src.opts.db)?;
            match start_collection(&src, store)? {
                Some(h) => { let _ = tokio::task::spawn_blocking(move || h.join()).await; }
                None => { let _ = tokio::signal::ctrl_c().await; }
            }
            Ok(())
        }
        Cmd::Mcp { db, mcp } => run_mcp(open_store(&db)?, &mcp).await,
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
        Cmd::Mark { name, note, at, db } => {
            let store = open_store(&db)?;
            let v = starlist::analysis::add_marker(&store, &name, note.as_deref(), at.as_deref(), starlist::analysis::now())
                .map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(())
        }
    }
}
