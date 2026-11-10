//! Docker: follow containers by name, or every container with a label, through
//! `docker logs -f --timestamps`. `STARLIST_DOCKER=podman` uses Podman instead.

use super::command::{self, Entry, Tool};
use super::{Start, rfc3339, split_timestamp};
use crate::collector::Pipeline;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub struct Docker {
    pub program: String,
}

impl Default for Docker {
    fn default() -> Self {
        Self {
            program: command::program("STARLIST_DOCKER", "docker"),
        }
    }
}

impl Tool for Docker {
    fn label(&self, target: &str) -> String {
        // Container IDs from a label listing are long; the first 12 characters
        // are what `docker ps` shows.
        let short = if target.len() == 64 && target.bytes().all(|b| b.is_ascii_hexdigit()) {
            &target[..12]
        } else {
            target
        };
        format!("docker container {short}")
    }

    fn command(&self, target: &str, start: Start) -> Command {
        let mut c = Command::new(&self.program);
        c.args(["logs", "--follow", "--timestamps"]);
        match start {
            Start::New => {
                c.args(["--tail", "0"]);
            }
            Start::All => {}
            Start::Since(t) => {
                c.args(["--since", &rfc3339(t)]);
            }
        }
        c.arg(target);
        c
    }

    fn decode(&self, line: &str, _stderr: bool) -> Option<Entry> {
        // Every container line has a timestamp; anything else is from docker.
        split_timestamp(line).map(|(t, rest)| Entry::new(t, rest))
    }
}

/// Follow one container by name or ID, across restarts and re-creation.
pub fn follow_container(
    pipeline: Arc<Pipeline>,
    name: String,
    first: Start,
    stop: &'static AtomicBool,
) {
    let tool = Docker::default();
    eprintln!("starlist: following {}", tool.label(&name));
    let alive = AtomicBool::new(true);
    let resume = Default::default();
    command::follow(&pipeline, &tool, &name, first, &resume, &alive, stop);
}

/// Follow every running container with this label (`KEY` or `KEY=VALUE`),
/// picking up containers started later.
pub fn follow_label(
    pipeline: Arc<Pipeline>,
    label: String,
    first: Start,
    stop: &'static AtomicBool,
) {
    let tool = Arc::new(Docker::default());
    let program = tool.program.clone();
    command::follow_all(
        pipeline,
        tool,
        move || {
            let mut c = Command::new(&program);
            c.args(["ps", "--quiet", "--no-trunc", "--filter"])
                .arg(format!("label={label}"));
            command::lines_of(c).map_err(|e| format!("listing containers with label {label}: {e}"))
        },
        first,
        stop,
    );
}
