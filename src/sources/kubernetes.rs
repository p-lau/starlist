//! Kubernetes: follow the pods of a deployment (or any workload with a label
//! selector), or every pod matching a selector, through `kubectl logs -f`. New
//! pods are picked up as they appear, so a rollout or a rescheduled pod is
//! followed from its first line, and a restarted container is followed again
//! where it left off. Uses the current kubectl context.

use super::command::{self, Entry, Tool};
use super::{Start, rfc3339, split_timestamp};
use crate::collector::Pipeline;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub struct Kubectl {
    pub program: String,
    pub namespace: Option<String>,
}

impl Kubectl {
    pub fn new(namespace: Option<String>) -> Self {
        Self {
            program: command::program("STARLIST_KUBECTL", "kubectl"),
            namespace,
        }
    }

    fn base(&self) -> Command {
        let mut c = Command::new(&self.program);
        if let Some(ns) = &self.namespace {
            c.args(["--namespace", ns]);
        }
        c
    }

    /// Turn a target into a label selector: `deployment/api` (also
    /// `statefulset/`, `daemonset/`, `replicaset/`, `job/`) is looked up,
    /// anything else is taken as a selector already, like `app=api`.
    pub fn selector(&self, target: &str) -> Result<String, String> {
        let Some((kind, name)) = target.split_once('/') else {
            return Ok(target.to_string());
        };
        let mut c = self.base();
        c.args([
            "get",
            kind,
            name,
            "--output=jsonpath={.spec.selector.matchLabels}",
        ]);
        let out = command::lines_of(c).map_err(|e| format!("looking up {target}: {e}"))?;
        selector_from_labels(&out.join("")).ok_or_else(|| {
            format!(
                "{target} has no matchLabels selector; pass a label selector like app=api instead"
            )
        })
    }
}

/// `{"app":"api","tier":"web"}` → `app=api,tier=web`.
pub fn selector_from_labels(json: &str) -> Option<String> {
    let v: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json).ok()?;
    let parts: Vec<String> = v
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|v| format!("{k}={v}")))
        .collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

impl Tool for Kubectl {
    fn label(&self, pod: &str) -> String {
        match &self.namespace {
            Some(ns) => format!("pod {ns}/{pod}"),
            None => format!("pod {pod}"),
        }
    }

    fn command(&self, pod: &str, start: Start) -> Command {
        let mut c = self.base();
        c.args([
            "logs",
            "--follow",
            "--timestamps",
            "--all-containers=true",
            "--ignore-errors=true",
            // `[pod/NAME/CONTAINER] ` in front of each line, so each container
            // is read as its own stream.
            "--prefix=true",
        ]);
        match start {
            Start::New => {
                c.arg("--tail=0");
            }
            Start::All => {}
            Start::Since(t) => {
                c.arg(format!("--since-time={}", rfc3339(t)));
            }
        }
        c.arg(pod);
        c
    }

    fn decode(&self, line: &str, stderr: bool) -> Option<Entry> {
        if stderr {
            return None;
        }
        let (container, line) = split_prefix(line);
        let (t, rest) = split_timestamp(line)?;
        let mut e = Entry::new(t, rest);
        e.stream = container.map(str::to_string);
        Some(e)
    }
}

/// Split kubectl's `--prefix` (`[pod/api-7d9f/app] `) off a line, returning the
/// container's name.
pub fn split_prefix(line: &str) -> (Option<&str>, &str) {
    if let Some(rest) = line.strip_prefix('[')
        && let Some((prefix, line)) = rest.split_once("] ")
        && let Some(container) = prefix.rsplit('/').next()
        && prefix.contains('/')
    {
        return (Some(container), line);
    }
    (None, line)
}

/// Follow every pod of `target` (a `deployment/NAME` or a label selector).
pub fn follow(
    pipeline: Arc<Pipeline>,
    target: String,
    namespace: Option<String>,
    first: Start,
    stop: &'static AtomicBool,
) {
    let tool = Arc::new(Kubectl::new(namespace));
    let lister = tool.clone();
    let mut selector: Option<String> = None;
    command::follow_all(
        pipeline,
        tool,
        move || {
            let sel = match &selector {
                Some(s) => s.clone(),
                None => {
                    let s = lister.selector(&target)?;
                    eprintln!("starlist: following the pods of {target} ({s})");
                    selector = Some(s.clone());
                    s
                }
            };
            let mut c = lister.base();
            c.args([
                "get",
                "pods",
                "--selector",
                &sel,
                "--field-selector=status.phase!=Pending",
                "--output=jsonpath={range .items[*]}{.metadata.name}{\"\\n\"}{end}",
            ]);
            command::lines_of(c).map_err(|e| format!("listing pods of {target}: {e}"))
        },
        first,
        stop,
    );
}
