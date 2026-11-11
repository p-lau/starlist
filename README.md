# starlist

Your applications are stars. starlist sends a small satellite to orbit each one: it watches the star's logs and sends back only what matters. For large fleets, an optional hub acts as the ground station that gathers every satellite's reports.

A small Rust collector that runs next to your app, watches its logs, and answers AI agents over MCP with **distilled, redacted summaries** instead of raw lines. It works with any language: it only reads the text your app already writes (plain lines or JSON, including Python and Java stack traces).

Release build: ~5 MB static binary, ~11 MB memory while serving.

## What the agent gets

| Tool | What it answers |
|---|---|
| `diagnose` | Start here. The likely causes of what is wrong now, ranked, in one call: why each stands out, the deploy it started with, its code location, sample lines, and the lines of a request that hit the top one. With nothing new, the busiest warnings and errors. |
| `whats_new` | Every pattern that appeared for the first time, came back, spiked, is slowly rising, or went quiet in the last window vs. a baseline. Spikes are judged against overall traffic, so a busy hour alone isn't flagged. A slow rise is a pattern whose share of the logs grew (over the window, or over the last 3 hours) by more than chance and more than it swung at any point in the day before. A spike that matches the same time on earlier days (a nightly job) is labelled `usual` and listed last instead of hidden. Errors first. |
| `verify_fix` | Did this error stop after the fix? Verdicts: `fixed`, `reduced`, `still_occurring`, `worse`, `too_early` (with minutes to wait), `appeared_after`, `not_active_before`. Warns when traffic dropped too much to tell. An error that keeps to certain hours isn't called fixed until those hours have come round. |
| `compare_deploy` | Everything that changed around a deploy (the latest one by default): new patterns (likely regressions), increased, stopped, decreased. |
| `list_patterns` / `get_pattern` | Find patterns by text or level; counts, hourly histogram, redacted samples with stack traces. |
| `get_context` | The redacted lines just before and after an error (or any time), so the agent sees what led up to it without the log file. |
| `get_request` | Every event of one request, in order: the path that led to an error. Findings carry an `example_request` ID to start from. |
| `set_pattern_state` | Resolve a pattern after a verified fix (it comes back as `regressed` if it occurs again), resolve it from the next release on, ignore it for a while or for a number of events, or reopen it. `timing` marks it as expected at set times (`usual`) or never usual (`always_alert`). |
| `get_release` | A release's report card: new, regressed and resolved patterns, and its error rate against the previous release. |
| `merge_patterns` | Merge patterns that are really one issue; the other IDs keep working. |
| `split_pattern` | Undo a merge: patterns merged by mistake count on their own again. |
| `add_marker` / `list_markers` | Name a point in time ("fix-currency"). Deploys are also detected from lines like `Starting api version 1.4.2`. |

When to reach for starlist: if a stack trace or clear symptom already points at recent code, reading the code is usually cheaper. starlist pays off when the symptom is unknown, nothing was deployed, the codebase is large, or the cause may be outside the code (an upstream service, data, config, clients), and it's cheaper than reading raw log files whenever logs are needed. The MCP instructions tell agents the same.

Every pattern carries `first_release`: the deploy that was live when it first appeared, so one `whats_new` call answers "which deploy started this error?". Patterns with a stack trace also carry `location`, the app's own frame closest to the failure (`shop/pricing.py:41 in price_cart`), for Python, Java/Kotlin, Node/TypeScript, Go and Rust panics; `get_pattern` lists every location a pattern was raised from, with counts. If the app runs from a different path than your checkout, say `/app` in a container, pass `--source-root /app` so paths match the repository.

Times accept `30m`, `2h`, an ISO time, or a marker name.

Tool definitions are sent with every model call an agent makes, so they cost tokens on every turn. `--tools core` offers only `diagnose`, `whats_new`, `get_pattern`, `get_context`, `get_request` and `verify_fix`, about half the definitions; `--tools diagnose,get_pattern` picks any set. The default is all of them.

Output is built for agent context. By default it's brief: compact JSON, relative times ("21m ago"), samples only on the top findings, and stack traces trimmed to your app's own frames plus the exception line (library frames are counted, not shown). Pass `detail: "full"` for absolute times, raw rates and untrimmed samples. On the demo, brief output is 40 to 60% smaller than before: `whats_new` is about 350 tokens, `get_pattern` about 200 and `verify_fix` about 115.

Every answer also has a hard size limit, however noisy the log: 8,000 characters brief (about 2k tokens) and 16,000 full, or whatever `max_chars` asks for (at least 1,000). To fit, starlist shortens samples, then drops them, then moves the last findings to the next page, and says what it cut in `trimmed`. `whats_new`, `compare_deploy` and `list_patterns` return a `next_cursor` when more remain; pass it back as `cursor` to get the next page of the same snapshot.

## How it works

1. **Collect**: follows files like `tail -F` (rotation-safe), reads stdin, follows Docker containers, journald units and Kubernetes pods, receives OpenTelemetry logs, or reads a Loki stream; groups stack-trace lines with the log line that produced them.
2. **Redact** before storing: emails, IPs, bearer tokens, JWTs, API keys (OpenAI, GitHub, Slack, Stripe, AWS), `password=`-style values, Luhn-valid card numbers, private keys, plus your own `--redact REGEX`.
3. **Pattern**: masks numbers, IDs, UUIDs, hex, quoted strings and timestamps; the pattern ID is a hash of that template, so it is the same across restarts and hosts. The exception type is part of the pattern (`... | KeyError`).
4. **Merge**: when patterns differ in just one word (a username, a hostname without digits) and that word has taken 5 different values, they are merged into one pattern with `<*>` there (`user=<*>` keeps the key). The oldest pattern's ID stays, and the merged IDs keep resolving to it, so IDs an agent already holds still work. Five values is high enough that status words like `started` / `failed` stay separate. Tune with `--merge-after N`, or `0` to turn it off. A merge that got it wrong can be undone with `split_pattern`; split patterns aren't merged automatically again.
5. **Group by rule** (optional): `--group 'connection (reset|refused) by (?P<name>\w+)'` puts every matching line into one pattern per `name` value, for errors that rule-based masking splits apart.
6. **Store**: SQLite (WAL) with per-10s counts, 5 recent samples per pattern, and markers. After 7 days (`--retention`) counts are folded into hourly counts, which are kept for 400 days (`--history`), so a month-over-month question still has an answer.
7. **Numbers** (optional): `--metric duration_ms` (repeatable) reads that field from each line, in `duration_ms=532`, `latency: 0.53` or JSON, and keeps an hourly histogram per pattern, so p50, p95 and p99 survive for months without the lines. Percentiles are within about 9%; count, mean, min and max are exact.

## Install

- **Prebuilt binary**: download the archive for your platform from the [latest release](https://github.com/p-lau/starlist/releases/latest) (Linux x86_64 and arm64, static; macOS Intel and Apple silicon; Windows x86_64), unpack it and put `starlist` on your `PATH`. `SHA256SUMS` lists the checksums.
- **Cargo**: `cargo install starlist` (dashboard included).
- **Container**: `ghcr.io/p-lau/starlist:1` for Linux amd64 and arm64. See [deploy/](deploy/) for a Docker Compose and a Kubernetes sidecar.

## Set it up

```sh
cd your-app
starlist init      # finds the logs, writes starlist.toml, sets up your agent
starlist serve     # reads starlist.toml; leave it running
```

`starlist init` looks for log files in the project (and `/var/log/<project>`), a Docker Compose project or running containers named after it, systemd units, Kubernetes workloads in your manifests, and an OpenTelemetry SDK in your dependencies. It lists what it found, lets you pick, and writes:

- `starlist.toml`: the sources, the database (`.starlist/starlist.db`, added to `.gitignore` if there is one), and source roots for code locations (the project directory, plus the Dockerfile's `WORKDIR`).
- The MCP settings of the clients the project already uses, or Claude Code's `.mcp.json` if none: `.mcp.json` (Claude Code), `.cursor/mcp.json`, `.vscode/mcp.json` and `.gemini/settings.json`, keeping any other servers in them. It prints the settings for the rest, Windsurf and Codex included.

Other MCP clients connect to `http://127.0.0.1:7341/mcp` (streamable HTTP) while `starlist serve` runs, or launch `starlist mcp --stdio` from the project directory themselves if they only speak stdio (it reads `starlist.toml` for the database).

It shows the plan before writing and asks before changing a file that exists. In CI or a script there's no prompt: new files are written, existing ones left alone, and `--yes` changes those too. `--dry-run` prints the files instead, `--client cursor,vscode` (or `none`) picks the clients, and `--stdio` has the agent start starlist itself instead of connecting to a running `starlist serve`.

`starlist.toml` holds command-line flags: each key is a flag without the dashes (`file = ["logs/app.log"]`, `from-start = true`). `serve`, `collect`, `mcp`, `ingest`, `export` and `mark` read it from the current directory, or from `--config PATH` (`STARLIST_CONFIG`); `--no-config` skips it. Flags typed on the command line win, lists add to the file's, and relative paths are read from the file's directory.

## Run it by hand

```sh
cargo build --release
# Follow a file and serve MCP at http://127.0.0.1:7341/mcp
./target/release/starlist serve --file /var/log/myapp.log
# Or pipe your app through it and still see your logs
npm start | ./target/release/starlist serve --stdin --tee
# Or let the agent launch it over stdio (reads the DB a collector writes)
./target/release/starlist collect --file app.log &
./target/release/starlist mcp --stdio
# Or read from Docker, journald, Kubernetes, OpenTelemetry or Loki
./target/release/starlist serve --docker shop-api
./target/release/starlist serve --k8s deployment/api --k8s-namespace shop
./target/release/starlist serve --otlp 127.0.0.1:4318
# Record the release without a log line: at startup...
STARLIST_VERSION=v1.4.3 ./target/release/starlist serve --file app.log
# ...or from a deploy script
./target/release/starlist mark v1.4.3 --deploy --note "fix currency KeyError"
```

Claude Code: `claude mcp add --transport http logs http://127.0.0.1:7341/mcp`

See [docs/sources.md](docs/sources.md) for every log source: Docker containers by name or label, journald units, the pods of a Kubernetes deployment across restarts, OTLP logs over HTTP (keeping trace IDs for `get_request`), and LogQL streams from Loki.

The dashboard, the hub server, the Prometheus exporter and each log source beyond files and stdin are Cargo features, all on by default. For just the MCP collector reading files and stdin (it can still push to a hub), build with `cargo build --release --no-default-features`, and add back what you need with `--features dashboard,hub,prometheus,docker,journald,kubernetes,otel,loki`.

The log engine (parsing, pattern IDs and merging, redaction, request IDs and code locations) is the `starlist-core` crate in `core/`, with no storage or server, so other tools can reuse it.

### Alerts

Agents don't have to poll. starlist exposes `whats_new` as the MCP resource `starlist://whats-new`; a client that subscribes gets `notifications/resources/updated` within seconds of a new, returning or spiking ERROR/FATAL pattern, then reads the resource (it includes `latest_alert`). One incident is one notification: patterns found together are gathered for 10 seconds, alerts are at least a minute apart, and a pattern isn't alerted again for 30 minutes.

Without a connected agent, pass `--notify-url https://...` (or `STARLIST_NOTIFY_URL`) to `serve`, `collect` or `mcp`, and each alert is POSTed there as JSON (`summary`, `findings`). `--no-alerts` turns both off.

### Dashboard

`starlist serve` also serves a small dashboard at `http://127.0.0.1:7341/`: log volume by level with deploys marked, what's new, a searchable pattern list with trends, each pattern's samples, locations, surrounding lines and requests, a "did the fix work?" check, and what changed around each deploy. It's about 50 KB, loads nothing from elsewhere, and uses the same token as the API. Release builds include it; see [dashboard/README.md](dashboard/README.md) to build it yourself. `--no-dashboard` turns it off.

### JSON API for your own UI

The same process serves a plain JSON API at `http://127.0.0.1:7341/api/v1`, for dashboards, scripts and custom UIs: status, event counts over time, `whats-new`, patterns, verify, compare, context lines, requests, markers, and a server-sent event stream of alerts. It returns data rather than agent summaries (ISO times, numbers, full samples) and has the same protection as `/mcp`. Calling it from a page on another origin needs `--cors-origin http://localhost:5173`; `--no-api` serves MCP only. See [docs/api.md](docs/api.md).

```sh
curl -s 'http://127.0.0.1:7341/api/v1/whats-new?window=1h&min_level=WARN'
```

### History, Prometheus and export

Each pattern costs about 45 bytes for each hour it occurs in, so a pattern seen every hour of a year takes about 400 KB, and most patterns are quiet most hours. Each `--metric` field adds about 200 to 800 bytes per pattern per hour it appears in. (Measured on 50 patterns over 30 days: 1.6 MB of hourly counts, 2 MB of histograms for 10 patterns with one field.)

- `--prometheus` serves `http://127.0.0.1:7341/metrics` for a Prometheus scraper: events per level, patterns per level and issue state, the percentiles of each `--metric` field over the last hour or so, and the database size. It has the same token and Host checks as the API.
- `starlist export --since 30d --db starlist.db > starlist.ndjson` writes patterns, hourly counts, hourly field percentiles and markers as one JSON object per line, for a warehouse, a spreadsheet or a backup.

### Hub: many services in one place

For more than one app, run a hub, the ground station, and point collectors at it. Every collector keeps working on its own: it still serves its own MCP, API and dashboard, and pushes a copy of what it records.

```sh
# On the hub host
starlist hub-token api --dir /var/lib/starlist-hub      # prints a push token for "api"
starlist hub-token pricing --dir /var/lib/starlist-hub
starlist hub --dir /var/lib/starlist-hub --http 0.0.0.0:7341

# Next to each app (replicas share one service name)
STARLIST_HUB_TOKEN=... starlist serve --file app.log \
  --hub https://hub.internal:7341 --service api --env production
```

- **What is pushed:** the events a collector records (already redacted and fingerprinted), its markers and its pattern merges. Nothing is pulled from collectors. Each push token can only write its own service and can't read anything; reading the hub uses the hub's own token, as on a collector.
- **Replicas** of a service add up in one place, because pattern IDs are the same on every host. A deploy reported by every replica is recorded once.
- **Outages:** a collector queues what it records in its own database and sends it in batches. While the hub is unreachable it keeps retrying and catches up afterwards. The queue is limited to `--hub-buffer` (default 200MB); beyond that the oldest batches are dropped and counted. A batch that is retried is applied only once.
- **Asking the hub:** the same MCP tools, API and dashboard, with a `service` argument (a picker in the dashboard). `list_services` lists them, and `whats_new` and `diagnose` without a service cover every service.
- **Requests across services:** `get_request` (and `GET /api/v1/requests/{id}`) without a service follows one request through every service in time order, tagged by service, with each service's first and last offset. Order across hosts is only as good as their clocks, and the answer says so. To fit the size limit, it keeps each service's first and last events and the errors longest.
- **Raw lines** at the hub are capped per service by `--raw-size`, and `--no-raw` keeps none. A collector run with `--no-raw` sends no lines for the hub's buffer either.
- The hub has no TLS of its own: put it behind a reverse proxy with HTTPS. A collector warns when it pushes to plain `http://` beyond localhost.

### CI: why did this run fail?

`starlist-ci` is a separate small binary for CI logs. A CI job starts fresh, so it has no history to compare with; instead, it compares the failing run's log with the last passing run's and reports what is new, what grew and what stopped, with line numbers, steps and code locations, as JSON for agents or Markdown for people. It keeps nothing on disk. A GitHub Action keeps each run's log and posts the comparison when a job fails. See [ci/README.md](ci/README.md).

```sh
starlist-ci --baseline passing.log failing.log
```

## Security, privacy and time zones

To report a vulnerability, see [SECURITY.md](SECURITY.md).

- Redaction happens before anything is stored, so what it recognizes never reaches the database, the tools, the API or a hub: emails, IPs, bearer tokens, JWTs, common API keys, `password=`-style values, card numbers and private keys, plus your own `--redact` rules.
- starlist has no TLS of its own. On one machine, keep the default `127.0.0.1` address. To reach it from elsewhere, listen on localhost or a private network and put a TLS-terminating proxy in front, for example Caddy with `reverse_proxy 127.0.0.1:7341`, or use an SSH tunnel (`ssh -L 7341:127.0.0.1:7341 host`). The same goes for a hub.
- On localhost (the default) no token is needed, and only loopback `Host` headers are accepted, which blocks DNS-rebinding tricks from a browser. The API also refuses writes from other sites' pages.
- When listening beyond localhost (`--http 0.0.0.0:7341`), a bearer token is required. Pass `--token` or set `STARLIST_TOKEN`. If you don't, one is generated and saved next to the database as `<db>.token` (owner-only permissions). Agents send it as `Authorization: Bearer <token>`, e.g. `claude mcp add --transport http logs http://host:7341/mcp --header "Authorization: Bearer $(cat starlist.token)"`.
- Request IDs are read from a W3C `traceparent` and fields like `trace_id`, `request_id`, `x-request-id` and `correlation_id` (any spelling, in JSON or `key=value` text). Add your own with `--request-key order_ref`. They're kept out of patterns, and an ID that looks like a secret is ignored. When an event has both, the trace ID wins, since it follows a request across services.
- Raw lines: to answer `get_context` and `get_request`, starlist keeps the most recent events in the database, redacted the same way as samples, up to `--raw-size` (default 20MB; oldest lines are dropped first). Redaction catches common secrets and personal data, but it can't know every sensitive value your app logs, so this buffer holds more of your log text than the pattern summaries do. If that's a concern, add `--redact` rules or run with `--no-raw`, which stores no raw lines and deletes any already kept.
- `--env production` (or `STARLIST_ENV`) labels what this collector watches; it shows in the release view, the API and the hub.
- Log timestamps without a zone are read as UTC. If your app logs local time, pass `--log-tz +02:00` (or `-0800`, `UTC`). Timestamps that carry a zone are never shifted.

## Footprint

- **Binary:** about 5 MB on Linux (static), about 7 MB on Windows.
- **Memory:** about 11 MB while serving a quiet app; 28 MB peak at 5,000 lines/s in the load test.
- **Database:** levelled at 63 MB at 5,000 lines/s with `--raw-size 16MB`. Most of it is the raw-line buffer, which `--raw-size` caps (or `--no-raw` removes). The rest is 10-second counts kept for `--retention` (7 days), hourly counts kept for `--history` (400 days), and the patterns themselves, which are never deleted (see [Known limits](#known-limits)).
- **Watching it live:** the dashboard footer and `GET /api/v1/status` (`health`) show the database size, memory (Linux), lines per second, new patterns in the last hour, and hub batches dropped. A steady stream of new patterns, or any dropped batches, is the sign to act.

## Demo

```sh
cargo run --release --example demo_log -- demo.log      # 26h of a fake checkout service: bad deploy 50m ago, fix 15m ago
./target/release/starlist serve --file demo.log --from-start --db demo.db &
cargo run --release --example ask -- http://127.0.0.1:7341/mcp whats_new '{"window":"1h","min_level":"WARN"}'
cargo run --release --example ask -- http://127.0.0.1:7341/mcp verify_fix '{"pattern_id":"<id from whats_new>","since":"v1.4.3"}'
```

## Tests

`cargo test --workspace`: unit tests (redaction, masking, stack-trace grouping, timestamps), scenario tests for every verdict with controlled time, and an end-to-end test that ingests logs and calls the tools through a real MCP client over HTTP. `tests/interface.rs` checks the tool names, arguments and output fields against [docs/tools.md](docs/tools.md).

`python bench/load.py` is the load test: it writes a busy log (requests, stack traces, high-cardinality values, rotation) into a file starlist follows, and checks that memory and the database level off and nothing is dropped. At 5,000 lines/s for 5 minutes (1.5M entries, 0.1% of them never-seen-before text), starlist kept up within seconds, peaked at 28 MB of memory, and the database levelled at 63 MB with `--raw-size 16MB`. It also runs weekly in CI.

## Known limits

- Lines without a timestamp get the time they were read.
- Patterns are kept for good, so a log where a large share of lines are unlike any other (free text with no IDs or numbers to mask) grows the pattern table without limit and slows ingestion: in the load test with 3% such lines at 5,000 lines/s, 35,000 patterns built up in 5 minutes and starlist fell behind. `--group` rules or `--merge-after` help for known shapes.
- No TLS of its own: see [Security](#security-privacy-and-time-zones).
- A hub has no push alerts or Prometheus endpoint of its own yet; each collector still has both.
