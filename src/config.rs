//! `starlist.toml`: command-line flags kept in a file.
//!
//! Each key is a flag's long name without `--`, so `file = ["app.log"]` is
//! `--file app.log` and `from-start = true` is `--from-start`. The file's flags
//! go before the ones typed on the command line, so a flag given there wins and
//! lists (`file`, `docker`, ...) add to the file's. Relative paths in `file`,
//! `db` and `source-root` are read from the file's directory, so the file works
//! from anywhere and can be committed with the project.

use anyhow::{Context, Result, bail};
use clap::ArgAction;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The file read from the current directory when no `--config` is given.
pub const FILE_NAME: &str = "starlist.toml";

/// Subcommands that read the config file.
pub const COMMANDS: &[&str] = &["serve", "collect", "mcp", "ingest", "export", "mark"];

/// Keys holding local paths, resolved against the config file's directory.
const PATH_KEYS: &[&str] = &["file", "db", "source-root"];

/// Insert the config file's flags into `args` (the full command line,
/// program name first) after the subcommand. Returns the file that was read.
///
/// The file is `--config PATH` (or `STARLIST_CONFIG`), else `starlist.toml` in
/// the current directory if there is one. `--no-config` reads none.
pub fn apply(cli: &clap::Command, args: &mut Vec<OsString>) -> Result<Option<PathBuf>> {
    // The subcommand: the first word that isn't a flag or --config's value.
    let mut pos = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].to_string_lossy();
        if a == "--config" {
            i += 2;
            continue;
        }
        if !a.starts_with('-') {
            pos = Some(i);
            break;
        }
        i += 1;
    }
    let Some(pos) = pos else {
        return Ok(None);
    };
    let sub_name = args[pos].to_string_lossy().to_string();
    if !COMMANDS.contains(&sub_name.as_str()) {
        return Ok(None);
    }
    let Some(sub) = cli.find_subcommand(&sub_name) else {
        return Ok(None);
    };
    if args.iter().any(|a| a == "--no-config") {
        return Ok(None);
    }
    let explicit = flag_value(&args[1..], "--config")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("STARLIST_CONFIG").map(PathBuf::from));
    let path = match explicit {
        Some(p) => p,
        None => {
            let p = PathBuf::from(FILE_NAME);
            if !p.is_file() {
                return Ok(None);
            }
            p
        }
    };
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let base = path
        .parent()
        .map(|d| {
            if d.as_os_str().is_empty() {
                Path::new(".")
            } else {
                d
            }
        })
        .unwrap_or(Path::new("."));
    let base = std::path::absolute(base).unwrap_or_else(|_| base.to_path_buf());
    let flags =
        to_args(cli, sub, &text, &base).with_context(|| format!("in {}", path.display()))?;
    args.splice(pos + 1..pos + 1, flags);
    Ok(Some(path))
}

/// The value of `--name VALUE` or `--name=VALUE`.
fn flag_value(args: &[OsString], name: &str) -> Option<OsString> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let s = a.to_string_lossy();
        if s == name {
            return it.next().cloned();
        }
        if let Some(v) = s.strip_prefix(name).and_then(|v| v.strip_prefix('=')) {
            return Some(v.into());
        }
    }
    None
}

/// Turn the file into flags for `sub`. Keys that `sub` doesn't take are
/// skipped (`http` means nothing to `mark`), but a key no command takes is an
/// error, so a typo doesn't go unnoticed.
pub fn to_args(
    cli: &clap::Command,
    sub: &clap::Command,
    text: &str,
    base: &Path,
) -> Result<Vec<OsString>> {
    let table: toml::Table = text.parse().context("not valid TOML")?;
    let mut out = vec![];
    for (key, value) in &table {
        if matches!(key.as_str(), "config" | "no-config") {
            bail!("`{key}` can't be set in the config file");
        }
        let Some(arg) = sub.get_arguments().find(|a| a.get_long() == Some(key)) else {
            let known = COMMANDS.iter().any(|c| {
                cli.find_subcommand(c)
                    .is_some_and(|s| s.get_arguments().any(|a| a.get_long() == Some(key)))
            });
            if known {
                continue;
            }
            bail!(
                "unknown key `{key}`; keys are the flags of `starlist serve --help` without the dashes"
            );
        };
        let flag = format!("--{key}");
        match arg.get_action() {
            ArgAction::SetTrue => match value {
                toml::Value::Boolean(true) => out.push(flag.into()),
                toml::Value::Boolean(false) => {}
                _ => bail!("`{key}` should be true or false"),
            },
            ArgAction::Append => {
                let items: Vec<&toml::Value> = match value {
                    toml::Value::Array(a) => a.iter().collect(),
                    v => vec![v],
                };
                for v in items {
                    out.push(flag.clone().into());
                    out.push(scalar(key, v, base)?);
                }
            }
            _ => {
                out.push(flag.into());
                out.push(scalar(key, value, base)?);
            }
        }
    }
    Ok(out)
}

fn scalar(key: &str, v: &toml::Value, base: &Path) -> Result<OsString> {
    let s = match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        _ => bail!("`{key}` should be a string, a number or a list of them"),
    };
    if PATH_KEYS.contains(&key) {
        return Ok(resolve(&s, base).into_os_string());
    }
    Ok(s.into())
}

/// A path from the file, read from the file's directory. Paths that are
/// absolute here or in a container (`/app`) are kept as written.
fn resolve(s: &str, base: &Path) -> PathBuf {
    let p = Path::new(s);
    if p.is_absolute() || s.starts_with('/') || s.starts_with('\\') {
        return p.to_path_buf();
    }
    let mut out = base.to_path_buf();
    for part in p.components() {
        match part {
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}
