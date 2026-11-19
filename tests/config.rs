//! `starlist.toml` handling: `config::apply` and `config::to_args`.

mod common;

use clap::{Arg, ArgAction, Command};
use starlist::config::{apply, to_args};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn cli() -> Command {
    Command::new("starlist")
        .subcommand(
            Command::new("serve")
                .arg(Arg::new("file").long("file").action(ArgAction::Append))
                .arg(Arg::new("http").long("http"))
                .arg(Arg::new("db").long("db"))
                .arg(Arg::new("max").long("max"))
                .arg(
                    Arg::new("from-start")
                        .long("from-start")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(Command::new("mark").arg(Arg::new("db").long("db")))
        .subcommand(Command::new("hub"))
}

/// A config file with `text` in a fresh temp directory.
fn write_config(text: &str) -> PathBuf {
    let dir = common::temp_db("starlist-config").with_extension("d");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("starlist.toml");
    std::fs::write(&p, text).unwrap();
    p
}

fn argv(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

fn strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// `words` with `--config <file with text>` right after the program name,
/// run through `apply`.
fn run(text: &str, words: &[&str]) -> Vec<String> {
    let path = write_config(text);
    let mut args = argv(words);
    args.insert(1, "--config".into());
    args.insert(2, path.into_os_string());
    apply(&cli(), &mut args).unwrap();
    strings(&args)
}

fn flags(text: &str) -> anyhow::Result<Vec<String>> {
    let cli = cli();
    let serve = cli.find_subcommand("serve").unwrap();
    Ok(strings(&to_args(&cli, serve, text, Path::new("/base"))?))
}

#[test]
fn file_values_become_flags_after_the_subcommand() {
    let out = run("http = \"0.0.0.0:9\"\nmax = 5\n", &["starlist", "serve"]);
    assert_eq!(&out[3..], ["serve", "--http", "0.0.0.0:9", "--max", "5"]);
}

#[test]
fn cli_flags_come_after_the_file_so_they_win() {
    let out = run("http = \"a:1\"\n", &["starlist", "serve", "--http", "b:2"]);
    assert_eq!(&out[3..], ["serve", "--http", "a:1", "--http", "b:2"]);
}

#[test]
fn bools_and_lists() {
    assert_eq!(flags("from-start = true").unwrap(), ["--from-start"]);
    assert!(flags("from-start = false").unwrap().is_empty());
    assert_eq!(
        flags("file = [\"/a\", \"/b\"]").unwrap(),
        ["--file", "/a", "--file", "/b"]
    );
    assert_eq!(flags("file = \"/a\"").unwrap(), ["--file", "/a"]);
    assert!(flags("from-start = \"yes\"").is_err());
    assert!(flags("http = [[1]]").is_err());
}

#[test]
fn relative_paths_resolve_against_the_file_directory() {
    let out = flags("file = \"app.log\"\ndb = \"/data/s.db\"\nhttp = \"./x\"").unwrap();
    // Keys come out in the file's (sorted) order: db, file, http.
    let file = Path::new(&out[3]);
    assert!(
        file.starts_with("/base") && file.ends_with("app.log"),
        "{out:?}"
    );
    assert_eq!(out[1], "/data/s.db");
    assert_eq!(out[5], "./x", "only path keys are resolved");
}

#[test]
fn errors_name_the_problem() {
    let e = flags("htpp = \"x\"").unwrap_err().to_string();
    assert!(e.contains("unknown key `htpp`"), "{e}");
    let e = flags("config = \"x\"").unwrap_err().to_string();
    assert!(e.contains("can't be set"), "{e}");
    let e = format!("{:#}", flags("http = ").unwrap_err());
    assert!(e.contains("not valid TOML"), "{e}");
}

#[test]
fn keys_other_commands_take_are_skipped() {
    let cli = cli();
    let mark = cli.find_subcommand("mark").unwrap();
    let out = to_args(&cli, mark, "http = \"x\"\ndb = \"/d\"", Path::new("/b")).unwrap();
    assert_eq!(strings(&out), ["--db", "/d"]);
}

#[test]
fn apply_leaves_args_alone_without_a_config() {
    let path = write_config("http = \"a:1\"");
    // --no-config, a command that doesn't read the file, and no subcommand.
    for words in [
        vec!["starlist", "serve", "--no-config"],
        vec!["starlist", "hub"],
        vec!["starlist", "--version"],
        vec!["starlist", "bogus"],
    ] {
        let mut args = argv(&words);
        args.insert(1, "--config".into());
        args.insert(2, path.clone().into_os_string());
        let before = args.clone();
        assert!(apply(&cli(), &mut args).unwrap().is_none(), "{words:?}");
        assert_eq!(args, before, "{words:?}");
    }
}

#[test]
fn apply_reports_missing_file_and_bad_content() {
    let mut args = argv(&["starlist", "--config", "/no/such/starlist.toml", "serve"]);
    let e = format!("{:#}", apply(&cli(), &mut args).unwrap_err());
    assert!(e.contains("reading"), "{e}");

    let path = write_config("nope = 1");
    let mut args = argv(&["starlist", "serve", "--config"]);
    args.push(path.into_os_string());
    let e = format!("{:#}", apply(&cli(), &mut args).unwrap_err());
    assert!(
        e.contains("unknown key `nope`") && e.contains("starlist.toml"),
        "{e}"
    );
}

#[test]
fn apply_returns_the_file_read() {
    let path = write_config("max = 1");
    let mut args = argv(&["starlist", "serve"]);
    args.insert(1, format!("--config={}", path.display()).into());
    assert_eq!(apply(&cli(), &mut args).unwrap(), Some(path));
}
