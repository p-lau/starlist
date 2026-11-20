# starlist

*Your app writes the logs. starlist tells your AI agent what matters in them.*

A small collector that runs next to your app, watches its logs, and answers AI agents over MCP with short, redacted summaries instead of raw lines. Works with any language: it only reads the text your app already writes.

One 5 MB binary, about 11 MB of memory, and secrets are redacted before anything is stored.

[Docs](https://starlist.app/docs) · [Install](#install) · [Tools](#what-your-agent-can-ask) · [Releases](https://github.com/p-lau/starlist/releases/latest)

---

## Before / after

Checkout is failing for some users. You ask your agent what's wrong.

**Before:** the agent opens the log. On the demo app that's 26 hours, 3,898 lines, about 75,000 tokens, and the error hides between the requests:

```
18:44:59 INFO  Starting checkout-service version 1.4.2 on port 8080
18:44:59 INFO  GET /api/cart/7174 200 in 22ms user=alice@example.com
18:45:04 INFO  GET /api/cart/4533 200 in 15ms user=alice@example.com
18:45:08 ERROR Unhandled error in POST /api/checkout cart=4631
Traceback (most recent call last):
  File "/app/checkout.py", line 88, in place_order
    total = price_cart(cart)
  File "/app/pricing.py", line 41, in price_cart
    currency = cart.meta['currency']
KeyError: 'currency'
18:45:10 INFO  GET /api/cart/1336 200 in 21ms user=bob@example.org
18:45:15 INFO  GET /api/cart/9245 200 in 14ms user=chen@example.net
... 3,880 more lines
```

**After:** the agent calls `diagnose` and gets the answer in about 500 tokens (shortened here):

```json
{
  "verdict": "Most likely a new ERROR pattern at /app/pricing.py:41 in price_cart (last 60m).",
  "causes": [
    {
      "kind": "new",
      "template": "ERROR Unhandled error in POST /api/checkout cart=<NUM> | KeyError",
      "why": "New: 27 in 60m (27/h), first seen 50m ago.",
      "started_with": "v1.4.2",
      "location": "/app/pricing.py:41 in price_cart",
      "samples": ["... currency = cart.meta['currency']\nKeyError: 'currency'"]
    },
    {
      "kind": "new",
      "template": "ERROR upstream payments timeout after <NUM> (attempt <NUM>)",
      "started_with": "v1.4.2"
    }
  ],
  "next": "Read the code at the location. get_pattern or get_request for more; after a fix, verify_fix."
}
```

The agent fixes `pricing.py` and ships v1.4.3. Then it calls `verify_fix`:

```json
{
  "verdict": "fixed",
  "before": "27 in 35m (46.2/h)",
  "after": "0 in 15m (0/h)",
  "explanation": "At the old rate about 12 would have occurred since the split; none did."
}
```

These are real answers from the [demo](#try-the-demo), which you can run yourself.

## Install

```sh
cargo install --locked starlist
```

Or grab a prebuilt binary from the [latest release](https://github.com/p-lau/starlist/releases/latest) (Linux, macOS, Windows), or run the container `ghcr.io/p-lau/starlist:1`. See [deploy/](deploy/) for Docker Compose and Kubernetes.

## Set it up

```sh
cd your-app
starlist init      # finds your logs, writes starlist.toml, connects your agent
starlist serve     # leave it running
```

That's it. `init` finds log files, Docker containers, systemd units, Kubernetes workloads and OpenTelemetry, asks what to watch, and adds starlist to the MCP settings of Claude Code, Cursor, VS Code or Gemini. It shows you every file before it writes it.

Any other MCP client can connect to `http://127.0.0.1:7341/mcp`, or run `starlist mcp --stdio`.

## What your agent can ask

| Tool | Answers |
|---|---|
| `diagnose` | What's wrong right now? Start here. |
| `whats_new` | What appeared, came back, spiked or went quiet since earlier? |
| `verify_fix` | Did the fix work? |
| `compare_deploy` | What changed around this deploy? |
| `get_pattern` / `list_patterns` | Tell me more about this error. |
| `get_context` / `get_request` | What happened just before it? What did this one request do? |
| `get_release` | How did this release do? |
| `set_pattern_state` | Mark this as resolved, ignored, or expected. |
| `merge_patterns` / `split_pattern` | These are the same issue. / No, they aren't. |
| `add_marker` / `list_markers` | Name a point in time, like a deploy. |

Every tool is in [docs/tools.md](docs/tools.md). Tool definitions cost tokens on every turn, so `--tools core` offers just the six most useful ones.

**When to use it:** if a stack trace already points at recent code, just read the code. starlist pays off when you don't know the cause yet, nothing was deployed, the codebase is big, or the problem may be outside the code.

## How it works

1. **Collect** from files, stdin, Docker, journald, Kubernetes, OpenTelemetry or Loki ([sources](docs/sources.md)).
2. **Redact** emails, IPs, tokens, API keys, passwords and card numbers before anything is stored.
3. **Group** similar lines into patterns. A pattern's ID stays the same across restarts and hosts.
4. **Store** counts in SQLite: every 10 seconds for a week, then hourly for 400 days. Patterns with no counts left are removed, and `--max-patterns` (default 10000) caps how many are kept.
5. **Answer** questions about those patterns, sized to fit an agent's context.

## Also included

- **Dashboard** at `http://127.0.0.1:7341/` for people.
- **JSON API** at `/api/v1` for your own scripts and UIs ([docs/api.md](docs/api.md)).
- **Alerts** pushed to a subscribed agent or a webhook (`--notify-url`).
- **Hub** to gather many services in one place, for when you run more than one app.
- **`starlist-ci`** to explain why a CI run failed, by comparing it with the last passing run ([ci/README.md](ci/README.md)).
- **Prometheus** metrics and NDJSON export.

All of it is covered in the [docs](https://starlist.app/docs).

## Try the demo

```sh
cargo run --release --example demo_log -- demo.log    # 26h of a fake shop: bad deploy, then a fix
cargo run --release -- serve --file demo.log --from-start
```

Open `http://127.0.0.1:7341/`, or point your agent at it and ask what went wrong.

## Security

- Listens on `127.0.0.1` by default. Beyond localhost it requires a bearer token.
- No TLS of its own: put a reverse proxy in front, or use an SSH tunnel.
- Redaction can't know every secret your app logs. Add `--redact` rules, or use `--no-raw` to keep no raw lines at all.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Known limits

- Lines without a timestamp get the time they were read.
- Patterns are kept forever, so logs full of one-off free text grow the pattern table without limit.
- A hub has no push alerts or Prometheus endpoint of its own yet.

## Development

```sh
cargo test --workspace
```

The log engine is the `starlist-core` crate in [core/](core/), usable on its own. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
