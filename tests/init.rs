//! `starlist init` and `starlist.toml` (issue #28).

use clap::{Arg, ArgAction, Command};
use starlist::init::{self, Client, Launch, Options, Prompt};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("starlist-init-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

/// Nothing installed.
fn no_tools(_: &str, _: &[&str]) -> Option<String> {
    None
}

/// Answers from a script; `None` once it runs out (as with no terminal).
struct Script {
    answers: Vec<Option<&'static str>>,
    asked: Vec<String>,
    said: String,
}

impl Script {
    fn new(answers: &[Option<&'static str>]) -> Self {
        Script {
            answers: answers.iter().rev().cloned().collect(),
            asked: vec![],
            said: String::new(),
        }
    }
}

impl Prompt for Script {
    fn ask(&mut self, q: &str) -> Option<String> {
        self.asked.push(q.to_string());
        self.answers.pop().flatten().map(str::to_string)
    }
    fn say(&mut self, text: &str) {
        self.said.push_str(text);
        self.said.push('\n');
    }
}

fn opts(dir: &Path) -> Options {
    Options {
        dir: dir.to_path_buf(),
        ..Default::default()
    }
}

#[test]
fn finds_logs_language_and_containers() {
    let dir = temp_dir("detect");
    write(
        &dir,
        "pyproject.toml",
        "[project]\ndependencies = [\"opentelemetry-sdk\"]\n",
    );
    write(&dir, "logs/app.log", "x\n");
    write(&dir, "logs/events.jsonl", "{}\n");
    write(&dir, "data/rows.jsonl", "{}\n");
    write(&dir, "npm-debug.log", "x\n");
    write(&dir, "node_modules/pkg/install.log", "x\n");
    write(&dir, ".cache/old.log", "x\n");
    write(
        &dir,
        "Dockerfile",
        "FROM python:3\nWORKDIR /app\nWORKDIR /srv/shop/\n",
    );
    write(&dir, "compose.yaml", "name: shop\nservices:\n  api: {}\n");
    write(&dir, "deploy/shop-worker.service", "[Service]\n");
    write(&dir, ".cursor/rules.md", "");

    let tools = |p: &str, a: &[&str]| -> Option<String> {
        match p {
            "docker" if a.contains(&"label=com.docker.compose.project=shop") => {
                Some("abc\ndef\n".into())
            }
            "systemctl" => Some("shop-worker.service loaded active running Worker\n".into()),
            _ => None,
        }
    };
    let d = init::detect(&dir, &tools);
    assert_eq!(d.languages, vec![("Python", "pyproject.toml".to_string())]);
    assert_eq!(d.workdir.as_deref(), Some("/srv/shop"));
    let got: Vec<(&str, &str, bool)> = d
        .candidates
        .iter()
        .map(|c| (c.key, c.value.as_str(), c.on))
        .collect();
    assert!(got.contains(&("file", "logs/app.log", true)));
    assert!(got.contains(&("file", "logs/events.jsonl", true)));
    assert!(!got.iter().any(|(_, v, _)| v.contains("rows")
        || v.contains("npm-debug")
        || v.contains("node_modules")
        || v.contains(".cache")));
    assert!(got.contains(&("docker-label", "com.docker.compose.project=shop", true)));
    assert!(
        d.candidates
            .iter()
            .any(|c| c.key == "docker-label" && c.why.contains("2 running"))
    );
    assert!(got.contains(&("journald", "shop-worker.service", true)));
    // Local logs were found, so OTLP is offered but not checked.
    assert!(got.contains(&("otlp", "127.0.0.1:4318", false)));
    assert_eq!(d.clients, vec![Client::Cursor]);
}

#[test]
fn kubernetes_only_when_nothing_local() {
    let dir = temp_dir("k8s");
    write(
        &dir,
        "k8s/app.yaml",
        "apiVersion: v1\nkind: Service\nmetadata:\n  name: api-svc\n---\napiVersion: apps/v1\n\
         kind: Deployment\nmetadata:\n  name: api\n  namespace: shop\nspec:\n  template:\n    \
         metadata:\n      name: ignored\n",
    );
    write(
        &dir,
        "chart/templates/d.yaml",
        "kind: Deployment\nmetadata:\n  name: {{ .Release.Name }}\n",
    );
    let kubectl = |p: &str, _: &[&str]| (p == "kubectl").then(String::new);
    let d = init::detect(&dir, &kubectl);
    assert_eq!(d.candidates.len(), 1);
    let c = &d.candidates[0];
    assert_eq!(
        (c.key, c.value.as_str(), c.on),
        ("k8s", "deployment/api", true)
    );
    assert_eq!(c.extra, Some(("k8s-namespace", "shop".to_string())));

    // Without kubectl it's listed but not used.
    let d = init::detect(&dir, &no_tools);
    assert!(!d.candidates[0].on);
}

#[test]
fn dockerfile_and_compose_parsing() {
    assert_eq!(init::dockerfile_workdir("WORKDIR $HOME/app\n"), None);
    assert_eq!(
        init::dockerfile_workdir("workdir \"/code\"\n").as_deref(),
        Some("/code")
    );
    assert_eq!(init::compose_name("services:\n  name: x\n"), None);
    assert_eq!(
        init::compose_name("name: 'shop'\n").as_deref(),
        Some("shop")
    );
}

#[test]
fn writes_config_and_client_without_a_terminal() {
    let dir = temp_dir("write");
    write(&dir, "go.mod", "module shop\n");
    write(&dir, "app.log", "x\n");
    write(&dir, ".gitignore", "/bin");
    let mut p = Script::new(&[]);
    let out = init::run(&opts(&dir), &no_tools, &mut p).unwrap();

    let cfg = read(&dir, "starlist.toml");
    assert!(cfg.contains("file = [\"app.log\"]"), "{cfg}");
    assert!(cfg.contains("db = \".starlist/starlist.db\""));
    assert!(cfg.contains("source-root = [\".\"]"));
    assert!(cfg.contains("# Detected: Go (go.mod)"));
    let mcp: serde_json::Value = serde_json::from_str(&read(&dir, ".mcp.json")).unwrap();
    assert_eq!(
        mcp["mcpServers"]["starlist"]["url"],
        "http://127.0.0.1:7341/mcp"
    );
    assert_eq!(mcp["mcpServers"]["starlist"]["type"], "http");
    // An existing file isn't changed when nobody can be asked.
    assert_eq!(read(&dir, ".gitignore"), "/bin");
    assert_eq!(out.skipped, vec![dir.join(".gitignore")]);
    assert!(
        p.said
            .contains("codex mcp add starlist --url http://127.0.0.1:7341/mcp")
    );

    // The config reads back as the flags it stands for.
    let flags = flags_for(&cfg, &dir);
    assert!(flags.contains(&format!("--file {}", dir.join("app.log").display())));
    assert!(flags.contains(&format!("--source-root {}", dir.display())));
}

#[test]
fn asks_before_changing_existing_files() {
    let dir = temp_dir("ask");
    write(&dir, "app.log", "x\n");
    write(&dir, "starlist.toml", "# mine\n");
    write(
        &dir,
        ".vscode/mcp.json",
        "{\"servers\": {\"other\": {\"command\": \"x\"}, \"starlist\": {\"url\": \"old\"}}}",
    );
    write(
        &dir,
        ".cursor/mcp.json",
        "{\"mcpServers\": {\"other\": {\"url\": \"u\"}}}",
    );

    // Enter for the sources, yes to the new and added files, no to both overwrites.
    let mut p = Script::new(&[Some(""), Some("y"), Some("n"), Some("")]);
    let o = Options {
        clients: Some(vec![Client::Cursor, Client::VsCode]),
        ..opts(&dir)
    };
    let out = init::run(&o, &no_tools, &mut p).unwrap();
    assert_eq!(read(&dir, "starlist.toml"), "# mine\n");
    assert!(read(&dir, ".vscode/mcp.json").contains("old"));
    let cursor: serde_json::Value = serde_json::from_str(&read(&dir, ".cursor/mcp.json")).unwrap();
    assert_eq!(cursor["mcpServers"]["other"]["url"], "u");
    assert_eq!(
        cursor["mcpServers"]["starlist"]["url"],
        "http://127.0.0.1:7341/mcp"
    );
    assert_eq!(out.written, vec![dir.join(".cursor/mcp.json")]);
    assert_eq!(p.asked.len(), 4, "{:?}", p.asked);
    assert!(p.asked[2].contains("starlist.toml already exists"));

    // --yes overwrites, keeping the other servers.
    let o = Options { yes: true, ..o };
    init::run(&o, &no_tools, &mut Script::new(&[])).unwrap();
    assert!(read(&dir, "starlist.toml").contains("file = [\"app.log\"]"));
    let vs: serde_json::Value = serde_json::from_str(&read(&dir, ".vscode/mcp.json")).unwrap();
    assert_eq!(vs["servers"]["other"]["command"], "x");
    assert_eq!(vs["servers"]["starlist"]["type"], "http");

    // A second run has nothing left to do.
    let mut p = Script::new(&[]);
    let out = init::run(&o, &no_tools, &mut p).unwrap();
    assert!(out.written.is_empty());
    assert!(p.said.contains("already set up"));
}

#[test]
fn picks_sources_by_number_and_dry_run_writes_nothing() {
    let dir = temp_dir("pick");
    write(&dir, "a.log", "x\n");
    write(&dir, "b.log", "x\n");
    let mut p = Script::new(&[Some("2"), Some("")]);
    init::run(
        &Options {
            clients: Some(vec![]),
            ..opts(&dir)
        },
        &no_tools,
        &mut p,
    )
    .unwrap();
    let cfg = read(&dir, "starlist.toml");
    let file_line = cfg.lines().find(|l| l.starts_with("file =")).unwrap();
    assert_eq!(file_line.matches(".log").count(), 1, "{cfg}");
    assert!(!dir.join(".mcp.json").exists());

    let dry = temp_dir("dry");
    write(&dry, "a.log", "x\n");
    let mut p = Script::new(&[]);
    init::run(
        &Options {
            dry_run: true,
            ..opts(&dry)
        },
        &no_tools,
        &mut p,
    )
    .unwrap();
    assert!(p.asked.is_empty());
    assert!(!dry.join("starlist.toml").exists());
    assert!(p.said.contains("--- starlist.toml ---"));
}

#[test]
fn nothing_found_still_writes_a_starting_point() {
    let dir = temp_dir("empty");
    init::run(&opts(&dir), &no_tools, &mut Script::new(&[])).unwrap();
    let cfg = read(&dir, "starlist.toml");
    assert!(cfg.contains("Nothing was found"));
    assert!(cfg.contains("# file = "));
    // Commented examples don't become flags.
    assert!(!flags_for(&cfg, &dir).contains("--file"));
}

#[test]
fn stdio_clients_start_starlist_with_the_config() {
    let launch = Launch::Stdio {
        command: "starlist".into(),
        args: vec![
            "serve".into(),
            "--stdio".into(),
            "--config".into(),
            "/p/starlist.toml".into(),
        ],
    };
    let e = Client::ClaudeCode.entry(&launch);
    assert_eq!(e["type"], "stdio");
    assert_eq!(e["args"][3], "/p/starlist.toml");
    assert_eq!(Client::Cursor.entry(&launch)["command"], "starlist");
    let s = init::snippets(&launch, &[]);
    assert!(
        s.contains("claude mcp add starlist -- starlist serve --stdio --config /p/starlist.toml")
    );
}

/// A small stand-in for the real CLI, with one flag of each kind.
fn cli() -> Command {
    let serve = Command::new("serve")
        .arg(Arg::new("file").long("file").action(ArgAction::Append))
        .arg(
            Arg::new("source-root")
                .long("source-root")
                .action(ArgAction::Append),
        )
        .arg(Arg::new("db").long("db"))
        .arg(Arg::new("http").long("http"))
        .arg(
            Arg::new("from-start")
                .long("from-start")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("merge-after").long("merge-after"));
    let mark = Command::new("mark").arg(Arg::new("db").long("db"));
    Command::new("starlist").subcommand(serve).subcommand(mark)
}

fn flags_for(text: &str, base: &Path) -> String {
    let cli = cli();
    let sub = cli.find_subcommand("serve").unwrap();
    let args = starlist::config::to_args(&cli, sub, text, base).unwrap();
    args.iter()
        .map(|a| a.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn config_keys_become_flags() {
    let base = Path::new("/proj");
    let text = "file = [\"logs/a.log\", \"/var/log/b.log\"]\ndb = \"x.db\"\n\
                source-root = [\".\", \"/app\"]\nfrom-start = true\nmerge-after = 0\nhttp = \"0.0.0.0:1\"\n";
    let f = flags_for(text, base);
    let a = base.join("logs").join("a.log");
    assert!(f.contains(&format!("--file {}", a.display())), "{f}");
    assert!(f.contains("--file /var/log/b.log"));
    assert!(f.contains(&format!("--db {}", base.join("x.db").display())));
    assert!(f.contains(&format!(
        "--source-root {} --source-root /app",
        base.display()
    )));
    assert!(f.contains("--from-start"));
    assert!(f.contains("--merge-after 0"));
    assert!(f.contains("--http 0.0.0.0:1"));
    assert!(!flags_for("from-start = false\n", base).contains("from-start"));

    // Keys for other commands are skipped; `mark` only takes the database.
    let cli = cli();
    let mark = cli.find_subcommand("mark").unwrap();
    let args = starlist::config::to_args(&cli, mark, text, base).unwrap();
    assert_eq!(args.len(), 2);

    let sub = cli.find_subcommand("serve").unwrap();
    let err = starlist::config::to_args(&cli, sub, "fiel = [\"a\"]\n", base).unwrap_err();
    assert!(err.to_string().contains("unknown key `fiel`"));
    assert!(starlist::config::to_args(&cli, sub, "from-start = \"yes\"\n", base).is_err());
}

#[test]
fn config_file_goes_after_the_subcommand() {
    let dir = temp_dir("apply");
    write(&dir, "s.toml", "db = \"d.db\"\n");
    let path = dir.join("s.toml");
    let mut args: Vec<OsString> = ["starlist", "--config", path.to_str().unwrap(), "mark", "v1"]
        .iter()
        .map(OsString::from)
        .collect();
    let used = starlist::config::apply(&cli(), &mut args).unwrap();
    assert_eq!(used, Some(path.clone()));
    let got: Vec<String> = args.iter().map(|a| a.to_string_lossy().into()).collect();
    assert_eq!(got[3], "mark");
    assert_eq!(got[4], "--db");
    assert_eq!(PathBuf::from(&got[5]), dir.join("d.db"));
    assert_eq!(got[6], "v1");

    // --no-config reads nothing.
    let mut args: Vec<OsString> = [
        "starlist",
        "mark",
        "--config",
        path.to_str().unwrap(),
        "--no-config",
        "v1",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    assert_eq!(starlist::config::apply(&cli(), &mut args).unwrap(), None);
}
