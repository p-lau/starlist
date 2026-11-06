# starlist

> Prototype.

Your applications are stars. starlist sends a small satellite to orbit each one: it watches the star's logs and sends back only what matters. For large fleets, an optional hub acts as the ground station that gathers every satellite's reports.

A small Rust collector that runs next to your app, watches its logs, and answers AI agents over MCP with **distilled, redacted summaries** instead of raw lines. It works with any language: it only reads the text your app already writes (plain lines or JSON, including Python and Java stack traces).

Release build: ~5 MB static binary, ~11 MB memory while serving.

## What the agent gets

| Tool | What it answers |
|---|---|
| `whats_new` | What appeared for the first time, came back, spiked, or went quiet in the last window vs. a baseline. Spikes are judged against overall traffic, so a busy hour alone isn't flagged. Errors first. |
| `verify_fix` | Did this error stop after the fix? Verdicts: `fixed`, `reduced`, `still_occurring`, `worse`, `too_early` (with minutes to wait), `appeared_after`, `not_active_before`. Warns when traffic dropped too much to tell. |
| `compare_deploy` | Everything that changed around a deploy (the latest one by default): new patterns (likely regressions), increased, stopped, decreased. |
| `list_patterns` / `get_pattern` | Find patterns by text or level; counts, hourly histogram, redacted samples with stack traces. |
| `get_context` | The redacted lines just before and after an error (or any time), so the agent sees what led up to it without the log file. |
| `get_request` | Every event of one request, in order: the path that led to an error. Findings carry an `example_request` ID to start from. |
| `add_marker` / `list_markers` | Name a point in time ("fix-currency"). Deploys are also detected from lines like `Starting api version 1.4.2`. |

Every pattern carries `first_release`: the deploy that was live when it first appeared, so one `whats_new` call answers "which deploy started this error?". Patterns with a stack trace also carry `location`, the app's own frame closest to the failure (`shop/pricing.py:41 in price_cart`), for Python, Java/Kotlin, Node/TypeScript, Go and Rust panics; `get_pattern` lists every location a pattern was raised from, with counts. If the app runs from a different path than your checkout, say `/app` in a container, pass `--source-root /app` so paths match the repository.

Times accept `30m`, `2h`, an ISO time, or a marker name.

Output is built for agent context. By default it's brief: compact JSON, relative times ("21m ago"), samples only on the top findings, and stack traces trimmed to your app's own frames plus the exception line (library frames are counted, not shown). Pass `detail: "full"` for absolute times, raw rates and untrimmed samples. On the demo, brief output is 40 to 60% smaller than before: `whats_new` is about 350 tokens, `get_pattern` about 200 and `verify_fix` about 115.

Every answer also has a hard size limit, however noisy the log: 8,000 characters brief (about 2k tokens) and 16,000 full, or whatever `max_chars` asks for (at least 1,000). To fit, starlist shortens samples, then drops them, then moves the last findings to the next page, and says what it cut in `trimmed`. `whats_new`, `compare_deploy` and `list_patterns` return a `next_cursor` when more remain; pass it back as `cursor` to get the next page of the same snapshot.

## How it works

1. **Collect**: follows files like `tail -F` (rotation-safe) or reads stdin; groups stack-trace lines with the log line that produced them.
2. **Redact** before storing: emails, IPs, bearer tokens, JWTs, API keys (OpenAI, GitHub, Slack, Stripe, AWS), `password=`-style values, Luhn-valid card numbers, private keys, plus your own `--redact REGEX`.
3. **Pattern**: masks numbers, IDs, UUIDs, hex, quoted strings and timestamps; the pattern ID is a hash of that template, so it is the same across restarts and hosts. The exception type is part of the pattern (`... | KeyError`).
4. **Merge**: when patterns differ in just one word (a username, a hostname without digits) and that word has taken 5 different values, they are merged into one pattern with `<*>` there (`user=<*>` keeps the key). The oldest pattern's ID stays, and the merged IDs keep resolving to it, so IDs an agent already holds still work. Five values is high enough that status words like `started` / `failed` stay separate. Tune with `--merge-after N`, or `0` to turn it off.
5. **Store**: SQLite (WAL) with per-10s counts, 5 recent samples per pattern, and markers. Counts kept 7 days by default.

## Run it

```sh
cargo build --release
# Follow a file and serve MCP at http://127.0.0.1:7341/mcp
./target/release/starlist serve --file /var/log/myapp.log
# Or pipe your app through it and still see your logs
npm start | ./target/release/starlist serve --stdin --tee
# Or let the agent launch it over stdio (reads the DB a collector writes)
./target/release/starlist collect --file app.log &
./target/release/starlist mcp --stdio
# Record the release without a log line: at startup...
STARLIST_VERSION=v1.4.3 ./target/release/starlist serve --file app.log
# ...or from a deploy script
./target/release/starlist mark v1.4.3 --deploy --note "fix currency KeyError"
```

Claude Code: `claude mcp add --transport http logs http://127.0.0.1:7341/mcp`

### Alerts

Agents don't have to poll. starlist exposes `whats_new` as the MCP resource `starlist://whats-new`; a client that subscribes gets `notifications/resources/updated` within seconds of a new, returning or spiking ERROR/FATAL pattern, then reads the resource (it includes `latest_alert`). One incident is one notification: patterns found together are gathered for 10 seconds, alerts are at least a minute apart, and a pattern isn't alerted again for 30 minutes.

Without a connected agent, pass `--notify-url https://...` (or `STARLIST_NOTIFY_URL`) to `serve`, `collect` or `mcp`, and each alert is POSTed there as JSON (`summary`, `findings`). `--no-alerts` turns both off.

### JSON API for your own UI

The same process serves a plain JSON API at `http://127.0.0.1:7341/api/v1`, for dashboards, scripts and custom UIs: status, event counts over time, `whats-new`, patterns, verify, compare, context lines, requests, markers, and a server-sent event stream of alerts. It returns data rather than agent summaries (ISO times, numbers, full samples) and has the same protection as `/mcp`. Calling it from a page on another origin needs `--cors-origin http://localhost:5173`; `--no-api` serves MCP only. See [docs/api.md](docs/api.md).

```sh
curl -s 'http://127.0.0.1:7341/api/v1/whats-new?window=1h&min_level=WARN'
```

### Security, privacy and time zones

- On localhost (the default) no token is needed, and only loopback `Host` headers are accepted, which blocks DNS-rebinding tricks from a browser. The API also refuses writes from other sites' pages.
- When listening beyond localhost (`--http 0.0.0.0:7341`), a bearer token is required. Pass `--token` or set `STARLIST_TOKEN`. If you don't, one is generated and saved next to the database as `<db>.token` (owner-only permissions). Agents send it as `Authorization: Bearer <token>`, e.g. `claude mcp add --transport http logs http://host:7341/mcp --header "Authorization: Bearer $(cat starlist.token)"`.
- Request IDs are read from a W3C `traceparent` and fields like `trace_id`, `request_id`, `x-request-id` and `correlation_id` (any spelling, in JSON or `key=value` text). Add your own with `--request-key order_ref`. They're kept out of patterns, and an ID that looks like a secret is ignored. See [docs/design/requests-across-services.md](docs/design/requests-across-services.md) for how this will work across services with the hub.
- Raw lines: to answer `get_context` and `get_request`, starlist keeps the most recent events in the database, redacted the same way as samples, up to `--raw-size` (default 20MB; oldest lines are dropped first). Redaction catches common secrets and personal data, but it can't know every sensitive value your app logs, so this buffer holds more of your log text than the pattern summaries do. If that's a concern, add `--redact` rules or run with `--no-raw`, which stores no raw lines and deletes any already kept.
- Log timestamps without a zone are read as UTC. If your app logs local time, pass `--log-tz +02:00` (or `-0800`, `UTC`). Timestamps that carry a zone are never shifted.

## Demo

```sh
cargo run --release --example demo_log -- demo.log      # 26h of a fake checkout service: bad deploy 50m ago, fix 15m ago
./target/release/starlist serve --file demo.log --from-start --db demo.db &
cargo run --release --example ask -- http://127.0.0.1:7341/mcp whats_new '{"window":"1h","min_level":"WARN"}'
cargo run --release --example ask -- http://127.0.0.1:7341/mcp verify_fix '{"pattern_id":"<id from whats_new>","since":"v1.4.3"}'
```

## Tests

`cargo test`: unit tests (redaction, masking, stack-trace grouping, timestamps), scenario tests for every verdict with controlled time, and an end-to-end test that ingests logs and calls the tools through a real MCP client over HTTP.

## Known limits (prototype)

- Lines without a timestamp get the time they were read.
- No TLS of its own: put it behind a reverse proxy (Caddy, nginx) or an SSH tunnel when it leaves the machine.
- No hub yet for many apps; collectors would push events to one optional hub.
