# Headless API, dashboard, and closing the gaps

Status: plan for v0.8.0 to v0.12.0, after push alerts (#12) and before 1.0.0
(#13). The hub is part of 1.0.0.

starlist answers agents well, but people compare it with hosted log and error
tools. Where it falls short today:

1. **No dashboards or metrics, and short history.** Counts are kept 7 days and
   the only way in is MCP.
2. **Requests stop at the service boundary.** `get_request` sees one collector;
   following a request across services needs the hub.
3. **Error grouping and release tracking are younger than Sentry's.** Patterns
   can't be resolved, ignored or split by hand, and there is no per-release
   view.

This note plans a headless HTTP API, a small dashboard built on it, and one
milestone for each gap. The ground rules don't change: one small binary, local
and private by default, nothing leaves the machine unless the user points it
somewhere.

## Milestones

| Milestone | What | Depends on |
|---|---|---|
| v0.8.0 | Headless JSON API next to MCP | |
| v0.9.0 | Dashboard, embedded and served by the binary | v0.8.0 |
| v0.10.0 | Issue states, regressions and a release view | v0.8.0 |
| v0.11.0 | Long-term history and metrics (Prometheus) | v0.8.0 |
| v0.12.0 | Hub: many collectors, requests across services | v0.8.0 to v0.11.0 |
| v1.0.0 | Release (#13); the API becomes stable | all |

The API comes first because the dashboard, the hub and custom UIs all read it.
Each later milestone adds its own API routes, MCP changes and dashboard view, so
nothing ships in one surface only.

## v0.8.0: headless API

Served by the same process and port as MCP, under `/api/v1`. It is for
programs (a custom UI, a script, a status page), so unlike MCP answers it
returns plain data: ISO 8601 UTC times, numbers instead of "27 in 35m" strings,
full sample text, and no character budget, only `limit` and `offset`.

| Route | Returns |
|---|---|
| `GET /api/v1/status` | Version, current release, pattern count, events and errors in the last hour, raw buffer size and age |
| `GET /api/v1/timeline?since=24h&step=&pattern_id=` | Event counts per level (or for one pattern) per time step, with markers in range |
| `GET /api/v1/whats-new?window=1h&baseline=24h&min_level=` | The `whats_new` findings: kind, counts and rates in window and baseline |
| `GET /api/v1/patterns?since=24h&min_level=&contains=&limit=&offset=` | Patterns with counts, most severe then most frequent first |
| `GET /api/v1/patterns/{id}` | One pattern: exception, locations, counts, hourly series, samples |
| `GET /api/v1/patterns/{id}/verify?since=&before=` | The `verify_fix` verdict |
| `GET /api/v1/compare?at=&before=` | The `compare_deploy` findings |
| `GET /api/v1/context?pattern_id=&at=&before=&after=` | Raw lines around an event |
| `GET /api/v1/requests/{id}` | One request's events in order |
| `GET /api/v1/markers` / `POST /api/v1/markers` | List or add markers and deploys |
| `GET /api/v1/alerts` | The latest push alert (#12), if any |
| `GET /api/v1/alerts/stream` | Server-sent events: one `alert` event per push alert, as it happens |

Times in query strings accept the same forms as MCP: `30m`, an ISO time, or a
marker name. Errors are `{"error": "..."}` with status 400 or 404.

MCP tools and API routes share one implementation. The analysis functions
compute typed results; MCP formats them briefly for agents, and the API
serializes them as they are. That keeps the verdicts identical and the code
small.

**Security.** The API is behind the same rules as `/mcp`:

- Beyond localhost a bearer token is required (the same `--token` or
  `<db>.token`).
- On localhost without a token only loopback `Host` headers are accepted, so a
  web page can't reach the API through DNS rebinding.
- `POST` needs `Content-Type: application/json`, and a request whose `Origin`
  is not this server or an allowed CORS origin is refused, so another site
  can't add markers through the browser.
- No CORS by default. `--cors-origin http://localhost:5173` (repeatable) lets a
  custom UI on another origin call the API.
- The API exposes what MCP already does, no more. `--no-api` turns it off for
  people who want MCP only.

The routes are versioned. Until 1.0.0 fields may still change; from 1.0.0
`/api/v1` only gains fields. A short reference goes in `docs/api.md`, with an
OpenAPI file if it stays cheap to maintain.

## v0.9.0: dashboard

**Recommendation: a separate SvelteKit app in `dashboard/`, built to static
files and embedded in the binary, served at `/`.**

Why embedded rather than a separate app people deploy:

- One thing to run. `starlist serve` then open `http://127.0.0.1:7341`. No Node
  on servers, no second process, no extra port, and the same auth.
- It stays local: the dashboard only talks to its own collector's API.
- It's small. A static Svelte build is about 100 to 150 KB, roughly 3% of the
  binary.

Why still a separate app in the source tree: it is built with the normal
SvelteKit tooling, it only uses the public API (so it doubles as the example
for custom UIs), and anyone can run it on its own with
`npm run dev -- --api http://host:7341`.

Build: `build.rs` embeds `dashboard/build` when it exists, and otherwise a
one-page placeholder that says how to build it, so `cargo build` never needs
Node. Release builds run `npm ci && npm run build` first. `--no-dashboard`
turns it off.

What it shows, in three screens:

1. **Overview.** Event rate per level over the last 24 hours with deploy
   markers, and the `whats_new` findings below it: new, returning, spiking,
   gone quiet. Errors first, as for agents.
2. **Patterns.** A searchable, filterable list (level, text, period) with a
   sparkline per row. A pattern page shows the template, exception, code
   locations, the hourly chart with markers, samples, and links to the context
   lines and the example request.
3. **Deploys.** Markers and deploys newest first; each opens its
   `compare_deploy` view, and a pattern can be checked against it with
   `verify_fix`.

It refreshes every 10 seconds while the tab is visible, and at once when the
alert stream reports something. Light and dark themes,
works at phone width, no external fonts or scripts. With a token, the
dashboard asks for it once and keeps it in session storage for that tab.

Read-only in v0.9.0 apart from adding markers. Resolve and ignore buttons come
with v0.10.0.

## v0.10.0: issue states, regressions and releases

Closes most of the gap with Sentry's issue workflow, without its backend.

- **States.** A pattern can be `open`, `resolved`, `resolved_in_next_release`
  or `ignored` (forever, for a duration, or until it passes a count). Set from
  the API, the dashboard, or a new MCP tool `set_pattern_state`, so an agent
  that verified a fix can resolve it.
- **Regressions.** A resolved pattern that occurs again is reopened and
  reported by `whats_new` as `regressed`, with the release it came back in.
  Ignored patterns are left out of `whats_new` and alerts until their condition
  ends.
- **Release view.** For each deploy: new, regressed and resolved patterns,
  error rate against the previous release, and time since deploy. MCP gets
  `get_release`; the dashboard gets a release page.
- **Grouping control.** Manual merge and split of patterns (the merge already
  exists internally with ID aliases), and `--group 'REGEX'` rules that pin
  lines with a matching fingerprint to one pattern.
- **Environment.** `--env production` tags a collector, so the hub and the
  release view can tell staging from production.

## v0.11.0: long-term history and metrics

starlist won't become a log store; raw logs stay in whatever archive you
already have. It can keep the shape of the logs much longer, at small cost.

- **Rollups.** Keep 10-second counts for `--retention` (7 days), then fold them
  into hourly counts kept for `--history` (default 400 days). That is at most
  one row per pattern per hour, so a year of a busy app is a few MB. Queries
  over long ranges read the hourly table automatically.
- **Metrics from logs.** Numbers named in a line (`duration_ms=532`,
  `"latency": 0.53`) are already masked out of templates. With
  `--metric duration_ms` starlist keeps a small histogram per pattern per hour
  for that field, so the API and dashboard can show p50, p95 and p99 latency
  per endpoint pattern without any instrumentation.
- **Prometheus.** `GET /metrics` (off unless `--metrics` is given) exports
  events per level, new patterns, active error patterns and the log-derived
  histograms, so existing Grafana or alerting setups can use starlist as a
  source. Labels stay low-cardinality: level and the top patterns only.
- **Export.** `starlist export --since 30d` writes patterns and counts as
  NDJSON for anyone who wants them elsewhere.

## v0.12.0: hub

The ground station from the README, and the answer to following requests
across services. Design for requests: [requests-across-services.md](requests-across-services.md).

- `starlist hub` is the same binary in another mode. Collectors run with
  `--hub https://hub:7341 --service api` and push distilled events: pattern
  rows, counts, markers, and (unless `--no-raw`) redacted raw lines with
  request IDs. Push is batched, retried, and authenticated with a per-collector
  token; nothing is pulled from collectors.
- Pattern IDs are already stable across hosts, so replicas of one service merge
  without coordination.
- The hub serves the same MCP tools, API and dashboard, with a `service`
  parameter on every route and a service filter in the UI. `get_request` on the
  hub returns one timeline across services.
- Collectors keep working on their own when the hub is down, and catch up
  within their retention when it returns.

The `service` field is reserved in the API from v0.8.0 so clients written
against a collector work against the hub.

## Out of scope

- Storing all raw logs long term, full-text search over history, or log
  shipping. Other tools do that well.
- Tracing instrumentation. starlist reads IDs that apps already log.
- Any hosted service. Everything here runs on the user's machines.
