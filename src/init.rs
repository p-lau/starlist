//! `starlist init`: find the app's logs, write `starlist.toml`, and set up the
//! agent's MCP client.
//!
//! Detection only looks: it reads the project's files and asks the local
//! `docker` and `systemctl` what is running. Nothing is written until the plan
//! has been shown, and existing files are only changed after a yes (or `--yes`).

use crate::config::FILE_NAME;
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where the database goes, relative to the project (kept out of git).
pub const DB_DIR: &str = ".starlist";
const DEFAULT_HTTP: &str = "127.0.0.1:7341";

/// Directories never searched for logs or manifests.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "venv",
    "env",
    "__pycache__",
    "dist",
    "build",
    "out",
    "bower_components",
    "coverage",
];
/// Log files that belong to tools, not the app.
const TOOL_LOGS: &[&str] = &[
    "npm-debug.log",
    "yarn-error.log",
    "yarn-debug.log",
    "pnpm-debug.log",
    "lerna-debug.log",
];
const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 20_000;
const MAX_FILES: usize = 10;

/// Runs a command and returns its stdout if it succeeded, or `None` if it
/// failed, isn't installed, or took too long. Swapped out in tests.
pub type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> Option<String>;

/// Run a local command with a time limit.
pub fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                std::io::Read::read_to_string(child.stdout.as_mut()?, &mut out).ok()?;
                return Some(out);
            }
            Ok(None) if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// A log source starlist could read, as a config key and value.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The flag, like "file" or "docker-label".
    pub key: &'static str,
    pub value: String,
    /// Why it was suggested, shown to the user.
    pub why: String,
    /// Used unless the user says otherwise.
    pub on: bool,
    /// A setting that goes with it, like the Kubernetes namespace.
    pub extra: Option<(&'static str, String)>,
}

#[derive(Debug, Default)]
pub struct Detected {
    /// Languages and the file that showed each.
    pub languages: Vec<(&'static str, String)>,
    /// The Dockerfile's WORKDIR: where the code lives in the container.
    pub workdir: Option<String>,
    pub candidates: Vec<Candidate>,
    /// MCP clients the project already has settings for.
    pub clients: Vec<Client>,
}

/// MCP clients with a settings file inside the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    Cursor,
    VsCode,
    Gemini,
}

impl Client {
    pub const ALL: [Client; 4] = [
        Client::ClaudeCode,
        Client::Cursor,
        Client::VsCode,
        Client::Gemini,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude-code",
            Client::Cursor => "cursor",
            Client::VsCode => "vscode",
            Client::Gemini => "gemini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::Cursor => "Cursor",
            Client::VsCode => "VS Code",
            Client::Gemini => "Gemini CLI",
        }
    }

    pub fn parse(s: &str) -> Option<Client> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude-code" | "claude" | "claudecode" => Some(Client::ClaudeCode),
            "cursor" => Some(Client::Cursor),
            "vscode" | "vs-code" | "copilot" => Some(Client::VsCode),
            "gemini" | "gemini-cli" => Some(Client::Gemini),
            _ => None,
        }
    }

    /// The settings file, relative to the project.
    pub fn file(self) -> &'static str {
        match self {
            Client::ClaudeCode => ".mcp.json",
            Client::Cursor => ".cursor/mcp.json",
            Client::VsCode => ".vscode/mcp.json",
            Client::Gemini => ".gemini/settings.json",
        }
    }

    fn servers_key(self) -> &'static str {
        match self {
            Client::VsCode => "servers",
            _ => "mcpServers",
        }
    }

    /// This client's entry for starlist.
    pub fn entry(self, launch: &Launch) -> Value {
        match (self, launch) {
            (Client::ClaudeCode | Client::VsCode, Launch::Http(url)) => {
                json!({"type": "http", "url": url})
            }
            (Client::Cursor, Launch::Http(url)) => json!({"url": url}),
            (Client::Gemini, Launch::Http(url)) => json!({"httpUrl": url}),
            (Client::ClaudeCode | Client::VsCode, Launch::Stdio { command, args }) => {
                json!({"type": "stdio", "command": command, "args": args})
            }
            (_, Launch::Stdio { command, args }) => json!({"command": command, "args": args}),
        }
    }

    /// Does the project already use this client?
    fn present(self, dir: &Path) -> bool {
        match self {
            Client::ClaudeCode => {
                dir.join(".mcp.json").is_file()
                    || dir.join(".claude").is_dir()
                    || dir.join("CLAUDE.md").is_file()
            }
            Client::Cursor => dir.join(".cursor").is_dir() || dir.join(".cursorrules").is_file(),
            Client::VsCode => dir.join(".vscode").is_dir(),
            Client::Gemini => dir.join(".gemini").is_dir() || dir.join("GEMINI.md").is_file(),
        }
    }
}

/// How the agent reaches starlist.
#[derive(Debug, Clone, PartialEq)]
pub enum Launch {
    /// A running `starlist serve` at this URL.
    Http(String),
    /// The agent starts starlist itself.
    Stdio { command: String, args: Vec<String> },
}

/// Look at the project in `dir`.
pub fn detect(dir: &Path, run: Runner) -> Detected {
    let mut d = Detected::default();
    let name = project_name(dir);
    let files = walk(dir);

    // Languages, from the files at the top of the project.
    let has = |f: &str| dir.join(f).exists();
    if has("Cargo.toml") {
        d.languages.push(("Rust", "Cargo.toml".into()));
    }
    if has("go.mod") {
        d.languages.push(("Go", "go.mod".into()));
    }
    if has("package.json") {
        let lang = if has("tsconfig.json") {
            "TypeScript"
        } else {
            "JavaScript"
        };
        d.languages.push((lang, "package.json".into()));
    }
    if let Some(f) = ["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"]
        .into_iter()
        .find(|f| has(f))
    {
        d.languages.push(("Python", f.into()));
    }
    if let Some(f) = ["build.gradle.kts", "build.gradle", "pom.xml"]
        .into_iter()
        .find(|f| has(f))
    {
        let kotlin = f.ends_with(".kts") || dir.join("src/main/kotlin").is_dir();
        d.languages
            .push((if kotlin { "Kotlin" } else { "Java" }, f.into()));
    }
    if has("Gemfile") {
        d.languages.push(("Ruby", "Gemfile".into()));
    }
    if has("composer.json") {
        d.languages.push(("PHP", "composer.json".into()));
    }
    if let Some(f) = files.iter().find(|f| {
        f.parent() == Some(dir)
            && f.extension()
                .is_some_and(|e| e == "csproj" || e == "sln" || e == "fsproj")
    }) {
        d.languages.push(("C#", file_name(f)));
    }
    d.workdir = std::fs::read_to_string(dir.join("Dockerfile"))
        .ok()
        .and_then(|t| dockerfile_workdir(&t));

    // Log files in the project, newest first.
    let mut logs: Vec<(&PathBuf, std::time::SystemTime)> = files
        .iter()
        .filter(|f| is_log_file(f, dir))
        .map(|f| {
            let t = f
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (f, t)
        })
        .collect();
    logs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let total = logs.len();
    for (f, _) in logs.into_iter().take(MAX_FILES) {
        d.candidates.push(Candidate {
            key: "file",
            value: relative(f, dir),
            why: "log file in the project".into(),
            on: true,
            extra: None,
        });
    }
    if total > MAX_FILES
        && let Some(c) = d.candidates.last_mut()
    {
        c.why = format!(
            "log file in the project ({} more not listed)",
            total - MAX_FILES
        );
    }
    // The system log directory, on Unix.
    if cfg!(unix) && name.len() >= 3 {
        let var = Path::new("/var/log");
        let mut found = vec![var.join(format!("{name}.log"))];
        if let Ok(rd) = std::fs::read_dir(var.join(&name)) {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "log"))
                .collect();
            v.sort();
            found.extend(v.into_iter().take(MAX_FILES));
        }
        for f in found.into_iter().filter(|f| f.is_file()) {
            d.candidates.push(Candidate {
                key: "file",
                value: f.to_string_lossy().into(),
                why: "log file under /var/log".into(),
                on: true,
                extra: None,
            });
        }
    }

    // Docker: a Compose project, or running containers named after the project.
    let compose = [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
    ]
    .into_iter()
    .find(|f| has(f));
    if let Some(cf) = compose {
        let text = std::fs::read_to_string(dir.join(cf)).unwrap_or_default();
        let project = compose_name(&text).unwrap_or_else(|| compose_project_name(&name));
        let label = format!("com.docker.compose.project={project}");
        let running = run(
            "docker",
            &["ps", "-q", "--filter", &format!("label={label}")],
        )
        .map(|o| o.lines().filter(|l| !l.trim().is_empty()).count());
        let why = match running {
            Some(n) => format!("services in {cf} ({n} running now)"),
            None => format!("services in {cf}"),
        };
        d.candidates.push(Candidate {
            key: "docker-label",
            value: label,
            why,
            on: true,
            extra: None,
        });
    } else if name.len() >= 3
        && let Some(out) = run("docker", &["ps", "--format", "{{.Names}}\t{{.Image}}"])
    {
        for line in out.lines() {
            let mut cols = line.split('\t');
            let (Some(cname), image) = (cols.next(), cols.next().unwrap_or("")) else {
                continue;
            };
            if cname.to_lowercase().contains(&name) || image.to_lowercase().contains(&name) {
                d.candidates.push(Candidate {
                    key: "docker",
                    value: cname.trim().to_string(),
                    why: format!("running container ({image})"),
                    on: true,
                    extra: None,
                });
            }
        }
    }

    // journald: units the project ships, and running units named after it.
    let known_units: Option<Vec<String>> = run(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--all",
            "--no-legend",
            "--plain",
        ],
    )
    .map(|o| {
        o.lines()
            .filter_map(|l| l.split_whitespace().next())
            .map(str::to_string)
            .collect()
    });
    let mut units: Vec<(String, String, bool)> = vec![];
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "service"))
    {
        let unit = file_name(f);
        let on = known_units.as_ref().is_some_and(|k| k.contains(&unit));
        let why = if on {
            format!("unit file {} (installed)", relative(f, dir))
        } else {
            format!("unit file {}", relative(f, dir))
        };
        units.push((unit, why, on));
    }
    if name.len() >= 3 {
        for u in known_units.iter().flatten() {
            if u.to_lowercase().contains(&name) && !units.iter().any(|(n, ..)| n == u) {
                units.push((u.clone(), "systemd unit on this machine".into(), true));
            }
        }
    }
    for (unit, why, on) in units {
        d.candidates.push(Candidate {
            key: "journald",
            value: unit,
            why,
            on,
            extra: None,
        });
    }

    // Kubernetes workloads in the project's manifests. Used only when nothing
    // local was found and kubectl is installed: the context may be a cluster
    // the user doesn't mean to watch from here.
    let local = d.candidates.iter().any(|c| c.on);
    let kubectl = !local && run("kubectl", &["version", "--client"]).is_some();
    let mut workloads = vec![];
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "yaml" || e == "yml"))
    {
        if f.metadata().map(|m| m.len()).unwrap_or(0) > 256 * 1024 {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(f) {
            for (target, ns) in k8s_workloads(&text) {
                if !workloads.iter().any(|(t, n, _)| *t == target && *n == ns) {
                    workloads.push((target, ns, relative(f, dir)));
                }
            }
        }
    }
    for (i, (target, ns, file)) in workloads.into_iter().enumerate() {
        d.candidates.push(Candidate {
            key: "k8s",
            value: target,
            why: format!("in {file}"),
            on: kubectl && i == 0,
            extra: ns.map(|n| ("k8s-namespace", n)),
        });
    }

    // OpenTelemetry: the app can send its logs over OTLP.
    let local = d.candidates.iter().any(|c| c.on);
    if let Some(f) = otel_dependency(dir) {
        d.candidates.push(Candidate {
            key: "otlp",
            value: "127.0.0.1:4318".into(),
            why: format!(
                "{f} uses OpenTelemetry; send its OTLP logs to http://127.0.0.1:4318/v1/logs"
            ),
            on: !local,
            extra: None,
        });
    }

    d.clients = Client::ALL.into_iter().filter(|c| c.present(dir)).collect();
    d
}

/// The project's name: its directory's, lowercased.
fn project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Compose's default project name for a directory name.
fn compose_project_name(dir_name: &str) -> String {
    dir_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .to_lowercase()
}

/// A top-level `name:` in a Compose file.
pub fn compose_name(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.strip_prefix("name:")?.trim().trim_matches(['"', '\'']);
        (!v.is_empty() && !v.contains('$')).then(|| v.to_string())
    })
}

/// The last WORKDIR of a Dockerfile, if it's a plain absolute path.
pub fn dockerfile_workdir(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l
                .get(..8)?
                .eq_ignore_ascii_case("WORKDIR ")
                .then(|| &l[8..])?;
            let v = rest.trim().trim_matches(['"', '\'']);
            (v.starts_with('/') && !v.contains('$')).then(|| v.trim_end_matches('/').to_string())
        })
        .next_back()
        .filter(|v| !v.is_empty())
}

/// Deployments, StatefulSets and DaemonSets in a YAML file, as `--k8s`
/// targets with their namespace. Helm templates are skipped.
pub fn k8s_workloads(text: &str) -> Vec<(String, Option<String>)> {
    if text.contains("{{") {
        return vec![];
    }
    let mut out = vec![];
    for doc in text.split("\n---") {
        let mut kind = None;
        let (mut name, mut ns) = (None, None);
        let mut in_meta = false;
        for line in doc.lines() {
            if !line.starts_with(' ') && !line.starts_with('\t') {
                in_meta = line.trim_end() == "metadata:";
                if let Some(k) = line.strip_prefix("kind:") {
                    kind = Some(k.trim().to_string());
                }
                continue;
            }
            if in_meta {
                let t = line.trim_start();
                let indent = line.len() - t.len();
                if indent != 2 {
                    continue;
                }
                if let Some(v) = t.strip_prefix("name:") {
                    name.get_or_insert(v.trim().trim_matches(['"', '\'']).to_string());
                } else if let Some(v) = t.strip_prefix("namespace:") {
                    ns.get_or_insert(v.trim().trim_matches(['"', '\'']).to_string());
                }
            }
        }
        let kind = match kind.as_deref() {
            Some("Deployment") => "deployment",
            Some("StatefulSet") => "statefulset",
            Some("DaemonSet") => "daemonset",
            _ => continue,
        };
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            out.push((format!("{kind}/{n}"), ns.filter(|n| !n.is_empty())));
        }
    }
    out
}

/// The dependency file that pulls in an OpenTelemetry SDK, if any.
fn otel_dependency(dir: &Path) -> Option<String> {
    [
        "package.json",
        "requirements.txt",
        "pyproject.toml",
        "go.mod",
        "Cargo.toml",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "Gemfile",
        "composer.json",
    ]
    .into_iter()
    .find(|f| {
        std::fs::read_to_string(dir.join(f))
            .is_ok_and(|t| t.to_lowercase().contains("opentelemetry"))
    })
    .map(str::to_string)
}

fn is_log_file(f: &Path, dir: &Path) -> bool {
    let name = file_name(f);
    if TOOL_LOGS.contains(&name.as_str()) {
        return false;
    }
    if name.ends_with(".log") {
        return true;
    }
    // JSON lines in a log directory.
    let in_log_dir = f
        .strip_prefix(dir)
        .ok()
        .and_then(|r| r.parent())
        .is_some_and(|p| {
            p.components()
                .any(|c| matches!(c.as_os_str().to_str(), Some("log" | "logs")))
        });
    in_log_dir && (name.ends_with(".jsonl") || name.ends_with(".ndjson"))
}

/// Files under `dir`, skipping hidden, dependency and build directories.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    let mut seen = 0;
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            seen += 1;
            if seen > MAX_ENTRIES {
                return out;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < MAX_DEPTH
                    && !name.starts_with('.')
                    && !SKIP_DIRS.contains(&name.as_str())
                {
                    stack.push((e.path(), depth + 1));
                }
            } else if ft.is_file() {
                out.push(e.path());
            }
        }
    }
    out
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// `p` relative to `dir` with forward slashes, for the config file.
fn relative(p: &Path, dir: &Path) -> String {
    match p.strip_prefix(dir) {
        Ok(r) => r
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => p.to_string_lossy().into(),
    }
}

/// A TOML string.
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_list(items: &[&str]) -> String {
    let v: Vec<String> = items.iter().map(|s| toml_str(s)).collect();
    format!("[{}]", v.join(", "))
}

/// The text of `starlist.toml` for the chosen sources.
pub fn config_text(d: &Detected, chosen: &[Candidate], http: &str) -> String {
    let mut t = String::new();
    t.push_str(
        "# starlist settings, written by `starlist init`.\n\
         # `starlist serve` (and collect, mcp, mark, export) reads ./starlist.toml, or --config PATH.\n\
         # Keys are the command-line flags without the dashes (see `starlist serve --help`).\n\
         # A flag given on the command line wins, and lists add to these.\n",
    );
    if !d.languages.is_empty() {
        let langs: Vec<String> = d
            .languages
            .iter()
            .map(|(l, f)| format!("{l} ({f})"))
            .collect();
        let _ = writeln!(t, "#\n# Detected: {}", langs.join(", "));
    }
    let _ = writeln!(
        t,
        "\n# Patterns, counts and recent lines (relative to this file; keep it out of git).\ndb = {}",
        toml_str(&format!("{DB_DIR}/starlist.db"))
    );

    t.push_str("\n# Where the logs come from.\n");
    if chosen.is_empty() {
        t.push_str(
            "# Nothing was found. Add a source, or pipe your app: `npm start | starlist serve --stdin --tee`\n\
             # file = [\"logs/app.log\"]\n\
             # docker = [\"my-container\"]\n\
             # docker-label = [\"com.docker.compose.project=myapp\"]\n\
             # journald = [\"myapp.service\"]\n\
             # k8s = [\"deployment/myapp\"]\n\
             # otlp = \"127.0.0.1:4318\"\n",
        );
    }
    let mut keys: Vec<&str> = vec![];
    for c in chosen {
        if !keys.contains(&c.key) {
            keys.push(c.key);
        }
    }
    for key in keys {
        let values: Vec<&str> = chosen
            .iter()
            .filter(|c| c.key == key)
            .map(|c| c.value.as_str())
            .collect();
        if key == "otlp" {
            let _ = writeln!(
                t,
                "# Point your app's OTLP log exporter at http://{}/v1/logs.\notlp = {}",
                values[0],
                toml_str(values[0])
            );
        } else {
            let _ = writeln!(t, "{key} = {}", toml_list(&values));
        }
    }
    if let Some((k, v)) = chosen.iter().find_map(|c| c.extra.clone()) {
        let _ = writeln!(t, "{k} = {}", toml_str(&v));
    }

    // Code locations.
    let supported = [
        "Rust",
        "Go",
        "TypeScript",
        "JavaScript",
        "Python",
        "Java",
        "Kotlin",
    ];
    let mut roots = vec!["."];
    if let Some(w) = &d.workdir {
        roots.push(w);
    }
    t.push_str(
        "\n# Code locations: stack-trace paths under these are shown relative to the repository\n",
    );
    match &d.workdir {
        Some(w) => {
            let _ = writeln!(
                t,
                "# (\".\" is this directory, {w} is the Dockerfile's WORKDIR in the container)."
            );
        }
        None => t.push_str(
            "# (\".\" is this directory; add the path the app runs from in a container).\n",
        ),
    }
    let unsupported: Vec<&str> = d
        .languages
        .iter()
        .map(|(l, _)| *l)
        .filter(|l| !supported.contains(l))
        .collect();
    if !unsupported.is_empty() && unsupported.len() == d.languages.len() {
        let _ = writeln!(
            t,
            "# Code locations are read from Python, Java/Kotlin, Node, Go and Rust traces, not {} yet.",
            unsupported.join(" or ")
        );
    }
    let _ = writeln!(t, "source-root = {}", toml_list(&roots));

    let _ = writeln!(
        t,
        "\n# MCP, the API and the dashboard at http://{http}/\nhttp = {}",
        toml_str(http)
    );
    t
}

/// What `init` was asked to do.
#[derive(Debug, Clone)]
pub struct Options {
    pub dir: PathBuf,
    /// Answer yes to everything, overwriting included.
    pub yes: bool,
    /// Show what would be written and write nothing.
    pub dry_run: bool,
    /// Clients to set up; `None` means the ones the project uses (or Claude Code).
    pub clients: Option<Vec<Client>>,
    /// Let the agent start starlist itself instead of connecting over HTTP.
    pub stdio: bool,
    pub http: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dir: PathBuf::from("."),
            yes: false,
            dry_run: false,
            clients: None,
            stdio: false,
            http: DEFAULT_HTTP.into(),
        }
    }
}

/// Questions for the user. `None` from `ask` means nobody can answer
/// (not a terminal), and the safe default is taken.
pub trait Prompt {
    fn ask(&mut self, question: &str) -> Option<String>;
    fn say(&mut self, text: &str);
}

/// Asks on the terminal, if there is one.
pub struct Terminal;

impl Prompt for Terminal {
    fn ask(&mut self, question: &str) -> Option<String> {
        use std::io::{IsTerminal, Write};
        if !std::io::stdin().is_terminal() {
            return None;
        }
        print!("{question} ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok()?;
        Some(line.trim().to_string())
    }
    fn say(&mut self, text: &str) {
        println!("{text}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The file doesn't exist yet.
    Create,
    /// Adds to an existing file without changing what's there.
    Add,
    /// Replaces an existing file or an existing starlist entry.
    Overwrite,
}

/// One file `init` wants to write.
#[derive(Debug, Clone)]
pub struct Write {
    pub path: PathBuf,
    pub content: String,
    pub change: Change,
    pub what: String,
}

/// What `init` did.
#[derive(Debug, Default)]
pub struct Outcome {
    pub written: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

/// Run `init`: detect, choose, show the plan, ask, write.
pub fn run(opts: &Options, run: Runner, prompt: &mut dyn Prompt) -> Result<Outcome> {
    let dir = std::path::absolute(&opts.dir)
        .with_context(|| format!("reading {}", opts.dir.display()))?;
    anyhow::ensure!(dir.is_dir(), "{} is not a directory", dir.display());
    let d = detect(&dir, run);
    let mut out = Outcome::default();

    let mut say = String::new();
    let _ = writeln!(say, "Looking at {}", dir.display());
    if !d.languages.is_empty() {
        let langs: Vec<&str> = d.languages.iter().map(|(l, _)| *l).collect();
        let _ = writeln!(say, "Language: {}", langs.join(", "));
    }
    if d.candidates.is_empty() {
        say.push_str("No logs found.\n");
    } else {
        say.push_str("Logs found:\n");
        for (i, c) in d.candidates.iter().enumerate() {
            let extra = c
                .extra
                .as_ref()
                .map(|(k, v)| format!(" --{k} {v}"))
                .unwrap_or_default();
            let _ = writeln!(
                say,
                "  [{}] {}. --{} {}{extra}   {}",
                if c.on { "x" } else { " " },
                i + 1,
                c.key,
                c.value,
                c.why
            );
        }
    }
    prompt.say(say.trim_end());

    let mut chosen: Vec<Candidate> = d.candidates.iter().filter(|c| c.on).cloned().collect();
    if !d.candidates.is_empty()
        && !opts.yes
        && !opts.dry_run
        && let Some(answer) = prompt.ask(
            "Use the checked ones? Press Enter, or type the numbers to use (like 1,3) or \"none\":",
        )
    {
        let a = answer.trim().to_lowercase();
        if a == "none" {
            chosen.clear();
        } else if !a.is_empty() && !matches!(a.as_str(), "y" | "yes") {
            chosen = a
                .split([',', ' '])
                .filter_map(|n| n.trim().parse::<usize>().ok())
                .filter_map(|n| d.candidates.get(n.wrapping_sub(1)).cloned())
                .collect();
        }
    }

    // The files to write.
    let config_path = dir.join(FILE_NAME);
    let config = config_text(&d, &chosen, &opts.http);
    let mut writes = vec![];
    match std::fs::read_to_string(&config_path) {
        Err(_) => writes.push(Write {
            path: config_path.clone(),
            content: config,
            change: Change::Create,
            what: "starlist settings".into(),
        }),
        Ok(old) if old == config => {}
        Ok(_) => writes.push(Write {
            path: config_path.clone(),
            content: config,
            change: Change::Overwrite,
            what: "starlist settings (replaces the existing file)".into(),
        }),
    }
    let launch = launch(opts, &config_path);
    let clients = opts.clients.clone().unwrap_or_else(|| {
        if d.clients.is_empty() {
            vec![Client::ClaudeCode]
        } else {
            d.clients.clone()
        }
    });
    let mut unwritable = vec![];
    for c in &clients {
        match client_write(&dir, *c, &launch) {
            Ok(Some(w)) => writes.push(w),
            Ok(None) => {}
            Err(e) => unwritable.push((*c, e)),
        }
    }
    if let Some(w) = gitignore_write(&dir) {
        writes.push(w);
    }

    // Show the plan, then ask.
    let mut plan = String::new();
    if writes.is_empty() {
        plan.push_str("\nEverything is already set up.");
    } else {
        plan.push_str(if opts.dry_run {
            "\nWould write:"
        } else {
            "\nWill write:"
        });
        for w in &writes {
            let verb = match w.change {
                Change::Create => "create",
                Change::Add => "add to",
                Change::Overwrite => "OVERWRITE",
            };
            let _ = write!(plan, "\n  {verb} {}   {}", relative(&w.path, &dir), w.what);
        }
    }
    for (c, e) in &unwritable {
        let _ = write!(
            plan,
            "\n  skip {}   {e}; add the entry below by hand",
            c.file()
        );
    }
    prompt.say(&plan);

    if opts.dry_run {
        for w in &writes {
            prompt.say(&format!(
                "\n--- {} ---\n{}",
                relative(&w.path, &dir),
                w.content.trim_end()
            ));
        }
        return Ok(out);
    }

    let mut go: Vec<bool> = vec![false; writes.len()];
    if opts.yes {
        go.fill(true);
    } else {
        let gentle: Vec<usize> = (0..writes.len())
            .filter(|i| writes[*i].change != Change::Overwrite)
            .collect();
        if !gentle.is_empty() {
            let yes = match prompt.ask("Write these? [Y/n]") {
                // Nobody to ask: create new files, leave existing ones alone.
                None => {
                    for i in &gentle {
                        go[*i] = writes[*i].change == Change::Create;
                    }
                    None
                }
                Some(a) => Some(!a.to_lowercase().starts_with('n')),
            };
            if let Some(yes) = yes {
                for i in &gentle {
                    go[*i] = yes;
                }
            }
        }
        for (i, w) in writes.iter().enumerate() {
            if w.change == Change::Overwrite {
                go[i] = prompt
                    .ask(&format!(
                        "{} already exists. Overwrite it? [y/N]",
                        relative(&w.path, &dir)
                    ))
                    .is_some_and(|a| a.to_lowercase().starts_with('y'));
            }
        }
    }
    for (w, go) in writes.iter().zip(go) {
        if !go {
            out.skipped.push(w.path.clone());
            continue;
        }
        if let Some(parent) = w.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&w.path, &w.content)
            .with_context(|| format!("writing {}", w.path.display()))?;
        out.written.push(w.path.clone());
    }

    let mut done = String::new();
    for p in &out.written {
        let _ = write!(done, "\nWrote {}", relative(p, &dir));
    }
    if !out.skipped.is_empty() {
        let list: Vec<String> = out.skipped.iter().map(|p| relative(p, &dir)).collect();
        let _ = write!(
            done,
            "\nLeft alone: {} (pass --yes to change existing files)",
            list.join(", ")
        );
    }
    done.push_str("\n\nNext:");
    match &launch {
        Launch::Http(url) => {
            let _ = write!(
                done,
                "\n  Start starlist here and leave it running: starlist serve\n  \
                 Your agent connects to {url}. Dashboard: {}",
                url.trim_end_matches("mcp")
            );
        }
        Launch::Stdio { .. } => {
            done.push_str("\n  Your agent starts starlist itself when it connects.");
        }
    }
    let written: Vec<Client> = clients
        .iter()
        .copied()
        .filter(|c| {
            out.written.iter().any(|p| p.ends_with(c.file()))
                || !writes.iter().any(|w| w.path.ends_with(c.file()))
                    && !unwritable.iter().any(|(u, _)| u == c)
        })
        .collect();
    if !written.is_empty() {
        let names: Vec<&str> = written.iter().map(|c| c.label()).collect();
        let _ = write!(
            done,
            "\n  Set up for {}; restart it or reload its MCP servers.",
            names.join(", ")
        );
    }
    done.push_str("\n\nOther MCP clients:");
    done.push_str(&snippets(&launch, &written));
    prompt.say(&done);
    Ok(out)
}

/// How the clients reach starlist.
fn launch(opts: &Options, config: &Path) -> Launch {
    if !opts.stdio {
        return Launch::Http(format!("http://{}/mcp", opts.http));
    }
    Launch::Stdio {
        command: own_command(),
        args: vec![
            "serve".into(),
            "--stdio".into(),
            "--config".into(),
            config.to_string_lossy().into(),
        ],
    }
}

/// "starlist" if this binary is the one on the PATH, else its full path.
fn own_command() -> String {
    let exe = std::env::current_exe().ok();
    let on_path = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|p| {
            let candidate = p.join(if cfg!(windows) {
                "starlist.exe"
            } else {
                "starlist"
            });
            candidate.is_file()
                && exe.as_ref().is_none_or(|e| {
                    std::fs::canonicalize(&candidate).ok() == std::fs::canonicalize(e).ok()
                })
        })
    });
    match exe {
        Some(e) if !on_path => e.to_string_lossy().into(),
        _ => "starlist".into(),
    }
}

/// The change to a client's settings file, `None` if it's already set up,
/// or an error if the file can't be read as JSON.
fn client_write(dir: &Path, c: Client, launch: &Launch) -> Result<Option<Write>, String> {
    let path = dir.join(c.file());
    let entry = c.entry(launch);
    let (mut root, change) = match std::fs::read_to_string(&path) {
        Err(_) => (Map::new(), Change::Create),
        Ok(text) if text.trim().is_empty() => (Map::new(), Change::Add),
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => (m, Change::Add),
            _ => return Err("not plain JSON (comments?)".into()),
        },
    };
    let servers = root
        .entry(c.servers_key())
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(format!("\"{}\" is not an object", c.servers_key()));
    };
    let change = match servers.get("starlist") {
        Some(old) if *old == entry => return Ok(None),
        Some(_) => Change::Overwrite,
        None => change,
    };
    servers.insert("starlist".into(), entry);
    let mut content = serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default();
    content.push('\n');
    let what = match change {
        Change::Overwrite => format!("{} (replaces its starlist server)", c.label()),
        _ => c.label().to_string(),
    };
    Ok(Some(Write {
        path,
        content,
        change,
        what,
    }))
}

/// Keep the database out of git, if the project has a .gitignore.
fn gitignore_write(dir: &Path) -> Option<Write> {
    let path = dir.join(".gitignore");
    let text = std::fs::read_to_string(&path).ok()?;
    let covered = text.lines().any(|l| {
        let l = l.trim().trim_start_matches('/').trim_end_matches('/');
        l == DB_DIR || l == "*.db"
    });
    if covered {
        return None;
    }
    let mut content = text.clone();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    let _ = writeln!(content, "{DB_DIR}/");
    Some(Write {
        path,
        content,
        change: Change::Add,
        what: format!("keeps {DB_DIR}/ out of git"),
    })
}

/// Settings for clients that weren't set up, to copy by hand.
pub fn snippets(launch: &Launch, skip: &[Client]) -> String {
    let mut s = String::new();
    let entry = |c: Client| {
        let mut m = Map::new();
        m.insert("starlist".into(), c.entry(launch));
        serde_json::to_string(&json!({ c.servers_key(): m })).unwrap_or_default()
    };
    if !skip.contains(&Client::ClaudeCode) {
        let cmd = match launch {
            Launch::Http(url) => format!("claude mcp add --transport http starlist {url}"),
            Launch::Stdio { command, args } => {
                format!("claude mcp add starlist -- {}", shell_join(command, args))
            }
        };
        let _ = write!(s, "\n  Claude Code:    {cmd}");
    }
    for c in [Client::Cursor, Client::VsCode, Client::Gemini] {
        if !skip.contains(&c) {
            let _ = write!(
                s,
                "\n  {:<15} {}  in {}",
                format!("{}:", c.label()),
                entry(c),
                c.file()
            );
        }
    }
    match launch {
        Launch::Http(url) => {
            let _ = write!(
                s,
                "\n  Windsurf:       {{\"mcpServers\":{{\"starlist\":{{\"serverUrl\":\"{url}\"}}}}}}  in ~/.codeium/windsurf/mcp_config.json\
                 \n  Codex:          codex mcp add starlist --url {url}"
            );
        }
        Launch::Stdio { command, args } => {
            let entry = json!({"mcpServers": {"starlist": {"command": command, "args": args}}});
            let _ = write!(
                s,
                "\n  Windsurf:       {entry}  in ~/.codeium/windsurf/mcp_config.json\
                 \n  Codex:          codex mcp add starlist -- {}",
                shell_join(command, args)
            );
        }
    }
    s
}

fn shell_join(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(|a| {
            if a.is_empty() || a.contains([' ', '"', '\'', '$']) {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
