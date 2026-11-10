# Changelog

Notable changes to starlist. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/); see [docs/versioning.md](docs/versioning.md) for what counts as a breaking change.

## [Unreleased]

### Added

- `starlist-ci`, a separate binary for CI logs: compares a failing run's log with the last passing run's and reports new, grown and stopped patterns with line numbers, steps and code locations, as JSON within the same size limits as the MCP tools, or as Markdown. Reads plain output, `gh run view --log` output and raw GitHub Actions logs, from a file or stdin, and writes nothing to disk. See [ci/README.md](ci/README.md).
- A GitHub Action (`p-lau/starlist/ci`) that keeps each run's log as an artifact and, when a job fails, writes the comparison to the job summary and optionally a pull request comment.
- Release archives for `starlist-ci`, and `starlist-ci` on crates.io.

## [1.0.0] - 2026-11-10

The first public release. Everything below is new.

### MCP tools

- `diagnose`, `whats_new`, `verify_fix`, `compare_deploy`, `list_patterns`, `get_pattern`, `get_context`, `get_request`, `set_pattern_state`, `get_release`, `merge_patterns`, `add_marker` and `list_markers`, with brief and full detail, a hard answer size limit and paging. Their arguments and output fields are listed in [docs/tools.md](docs/tools.md).
- `--tools core` or any list of tools, to offer agents fewer definitions.
- Errors that recur at the same time of day are marked `usual` and not alerted on, `verify_fix` waits for a pattern's usual hours, and `set_pattern_state` takes a `timing` override.

### Collecting

- Files (rotation-safe), stdin with `--tee`, Docker containers by name or label, journald units, Kubernetes workloads, OTLP logs over HTTP and Loki streams.
- Plain and JSON lines, multi-line stack traces for Python, Java/Kotlin, Node/TypeScript, Go and Rust, request IDs and trace IDs, code locations with `--source-root`, and `--log-tz` for local-time logs.
- Redaction of common secrets and personal data before anything is stored, plus `--redact` rules.
- Stable pattern IDs, automatic merging of near-identical patterns, `--group` rules, and `--metric` percentiles.
- Deploy detection from log lines, `STARLIST_VERSION` and `starlist mark`.

### Running it

- `starlist init` to find a project's logs and set up its agents, and `starlist.toml` for settings.
- `serve`, `collect`, `mcp --stdio`, `ingest`, `export` and `mark`.
- MCP over streamable HTTP and stdio, with a bearer token required beyond localhost.
- A JSON API at `/api/v1` ([docs/api.md](docs/api.md)), an embedded dashboard, a Prometheus endpoint, alerts, and a hub that gathers many collectors.
- Seven days of 10-second counts and 400 days of hourly history, with a bounded buffer of redacted raw lines (`--raw-size`, `--no-raw`).
- The log engine as a separate crate, `starlist-core`.

[Unreleased]: https://github.com/p-lau/starlist/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/p-lau/starlist/releases/tag/v1.0.0
