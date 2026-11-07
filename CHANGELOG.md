# Changelog

Notable changes to starlist. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/); see [docs/versioning.md](docs/versioning.md) for what counts as a breaking change.

## [Unreleased]

The first public release. Everything below is new.

### MCP tools

- `diagnose`, `whats_new`, `verify_fix`, `compare_deploy`, `list_patterns`, `get_pattern`, `get_context`, `get_request`, `set_pattern_state`, `get_release`, `merge_patterns`, `add_marker` and `list_markers`, with brief and full detail, a hard answer size limit and paging. Their arguments and output fields are listed in [docs/tools.md](docs/tools.md).
- `--tools core` or any list of tools, to offer agents fewer definitions.

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

[Unreleased]: https://github.com/p-lau/starlist/commits/main
