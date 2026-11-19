# starlist

starlist is a small collector that watches an app's logs and serves distilled, redacted summaries to AI agents over MCP. MIT licensed and public: keep commits, PRs and issues free of personal details, attribution lines and session or project links.

## Repository map

| Path | What it is |
|---|---|
| `Cargo.toml` | Workspace root (edition 2024, shared version) and the `starlist` package |
| `src/` | `starlist` binary. Entry point `src/main.rs` (commands `init`, `serve`, `collect`, `mcp`, `ingest`, `export`, `hub`, `hub-token`, `mark`). MCP server, SQLite store, HTTP API, alerts, hub, Prometheus, log sources (`src/sources/`) |
| `core/` | `starlist-core` library: parsing, pattern IDs and merging, redaction, request IDs, code locations. No storage, server or async runtime |
| `ci/` | `starlist-ci` binary (`ci/src/main.rs`) and its GitHub Action (`ci/action.yml`): compares a failing CI log with the last passing one |
| `build.rs` | Embeds `dashboard/build/` into the binary when the `dashboard` feature is on and the dashboard has been built |
| `dashboard/` | SvelteKit (Svelte 5) dashboard served at `/`, built statically |
| `site/` | Landing page at starlist.app (SvelteKit, deployed to Cloudflare Workers from `main`, watch path `site/*`) |
| `tests/` | Integration tests for `starlist`; `ci/tests/` for `starlist-ci` |
| `examples/` | `demo_log.rs` (generates a realistic demo log) and `ask.rs` (example MCP client) |
| `docs/` | `api.md`, `tools.md`, `sources.md`, `versioning.md`, `releasing.md`, design notes in `docs/design/` |
| `deploy/` | Dockerfiles, compose and Kubernetes manifests |
| `bench/` | Python agent benchmark. Costly: do not run unless asked |
| `.github/workflows/` | `ci.yml` (checks), `release.yml` (tag-triggered release), `audit.yml`, `load.yml` |

Cargo features (all on by default): `dashboard`, `hub`, `prometheus`, `docker`, `journald`, `kubernetes`, `otel`, `loki`. Tests that need a feature are declared as `[[test]]` with `required-features` in `Cargo.toml`.

## Setup

- Stable Rust (CI uses `dtolnay/rust-toolchain@stable` with clippy and rustfmt). No MSRV is pinned.
- Node 22 (`dashboard/.nvmrc`), only for the dashboard and site. Without a built dashboard the binary serves a page saying how to build it, so Rust work never needs Node.
- On Windows, use Git Bash for shell scripts.

```sh
(cd dashboard && npm ci && npm run build)   # optional: embed the dashboard
(cd site && npm ci)                          # only for site work
cargo build
```

Try a change against a realistic log:

```sh
cargo run --example demo_log -- demo.log
cargo run -- serve --file demo.log --from-start   # http://127.0.0.1:7341/
```

## Checks

The same checks CI runs (`.github/workflows/ci.yml`, `CONTRIBUTING.md`). All must pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings
cargo test --workspace --locked --no-default-features
```

When `dashboard/` or `site/` changes, also run in that folder:

```sh
npm ci && npm run check && npm run build
```

Baseline (2026-11-19, commit b66a318): all of the above pass. 156 tests with default features, 137 with `--no-default-features`, 0 Svelte check errors in either app. CI also builds and smoke-tests the Docker image (`deploy/Dockerfile`).

While fixing a failure, re-run only the failing tests; run the full suite once at the end.

## Completion rules

A change is done when:

1. It does one thing, within its assignment, and touches only the files that change needs.
2. Tests are added or updated for it. Redaction changes include a value that should be caught and one that should not.
3. Every check above passes, with default features and with `--no-default-features`, and the Svelte checks pass for any touched app.
4. `README.md`, `docs/api.md` or `docs/tools.md` are updated when flags, tools, API or output change. Tool names and output fields are a public interface: change them only when the contract says so.
5. `CHANGELOG.md` has an entry under `[Unreleased]` for user-visible changes.
6. It is committed on a branch (never `main`) with a short imperative message and no attribution trailers, names or links.
7. The worker reports: changed files, commit SHA, checks performed with exit codes, and unresolved blockers.

Never push, tag, publish to crates.io or ghcr, or deploy the site unless the person asks.

## Agents

Project agents live in `.claude/agents/`: `architect` plans contracts and assignments, `backend-engineer` and `frontend-engineer` implement, `test-engineer` writes acceptance tests and verifies, `code-reviewer` reviews read-only, `release-engineer` integrates commits and drafts the PR. The architect and reviewer run on `opus`; the other four run on `sonnet`. Experimental Agent Teams stays disabled in `.claude/settings.json`; agents run as ordinary subagents.
