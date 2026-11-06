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
| `compare_deploy` | Everything that changed around a deploy: new patterns (likely regressions), increased, stopped, decreased. |
| `list_patterns` / `get_pattern` | Find patterns by text or level; counts, hourly histogram, redacted samples with stack traces. |
| `add_marker` / `list_markers` | Name a point in time ("fix-currency"). Deploys are also detected from lines like `Starting api version 1.4.2`. |

Times accept `30m`, `2h`, an ISO time, or a marker name.

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
# Mark a deploy from a script
./target/release/starlist mark v1.4.3 --note "fix currency KeyError"
```

Claude Code: `claude mcp add --transport http logs http://127.0.0.1:7341/mcp`

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

- Timestamps without a timezone are read as UTC; lines without timestamps get the time they were read.
- HTTP binds to localhost with no auth. Add tokens/TLS before exposing it.
- No hub yet for many apps; collectors would push events to one optional hub.
