# JSON API

starlist serves a JSON API next to MCP, on the same address:
`http://127.0.0.1:7341/api/v1`. It answers the same questions as the MCP tools,
for programs instead of agents: a dashboard, a script, a status page, your own
UI.

- Times are ISO 8601 in UTC with milliseconds (`2026-11-06T11:00:00.000Z`).
- Counts and rates are numbers; rates are per hour.
- Text is the full redacted sample or line (up to 8,000 characters), with no
  stack-trace trimming.
- Lists page with `limit` and `offset` and say how many there are in `total`.
- Errors are `{"error": "..."}` with status 400 (bad input), 404 (unknown
  pattern, request or route) or 500.
- Query parameters that take a time accept what MCP does: a duration back from
  now (`30m`, `2h`, `1d`), an ISO time, or a marker name such as `v1.4.3`.
- `service` names the service on a hub, and is `null` on a single collector. See [On a hub](#on-a-hub).

Until 1.0.0 fields may still change. From 1.0.0, `/api/v1` only gains fields; see [versioning.md](versioning.md).

## Access

The API has the same protection as `/mcp`:

- **Localhost, no token** (the default): only requests whose `Host` is
  loopback are answered, which blocks DNS rebinding.
- **Beyond localhost**: every request needs `Authorization: Bearer <token>`
  (`--token`, `STARLIST_TOKEN`, or the generated `<db>.token`).
- **Writes** (`POST`) need `Content-Type: application/json`, and a browser
  request whose `Origin` is another site is refused, so a web page can't add
  markers through a visitor's browser.
- **CORS** is off. To call the API from a UI on another origin, start starlist
  with `--cors-origin http://localhost:5173` (repeatable). Preflights are
  answered without a token; the actual requests still need it.
- `--no-api` serves MCP only.
- `--prometheus` adds `/metrics` (Prometheus text format) with the same checks.

```sh
curl -s http://127.0.0.1:7341/api/v1/status
curl -s -H "Authorization: Bearer $(cat starlist.token)" http://host:7341/api/v1/status
```

## Routes

### `GET /status`

```json
{
  "version": "0.1.0",
  "now": "2026-11-06T12:00:00.000Z",
  "service": null,
  "env": "production",
  "current_release": {"name": "v1.4.3", "at": "2026-11-06T11:45:00.000Z"},
  "patterns": 42,
  "last_hour": {"events": 9120, "errors": 27},
  "raw": {"events": 51234, "oldest": "2026-11-05T09:12:00.000Z"},
  "alerts": true,
  "health": {
    "db_bytes": 66060288,
    "memory_bytes": 29360128,
    "events_per_sec": 152.0,
    "new_patterns_last_hour": 2,
    "hub_dropped": 0,
    "retention_secs": 86400,
    "history_secs": 2592000
  }
}
```

`health` shows what starlist itself costs and whether it is keeping up:
`db_bytes` is the database on disk with its write-ahead log; `memory_bytes` is
the process's resident memory (Linux only, `null` elsewhere);
`events_per_sec` is the last minute's rate; `new_patterns_last_hour` climbing
steadily means many lines don't group (see Known limits in the README);
`hub_dropped` counts queued batches dropped while a hub was unreachable;
`retention_secs` and `history_secs` are the settings in effect (`null` on a hub).

### `GET /timeline`

Event counts over time, per level or for one pattern, with the markers in range.

| Parameter | Default | |
|---|---|---|
| `since` | `24h` | Start |
| `until` | now | End |
| `step` | about 100 points | Seconds or a duration like `5m`; at least 10s, at most 5,000 points |
| `pattern_id` | | Count one pattern instead of all levels |

```json
{
  "from": "2026-11-05T12:00:00.000Z",
  "to": "2026-11-06T12:00:00.000Z",
  "step_seconds": 900,
  "series": [
    {"level": "ERROR", "counts": [0, 0, 3, 12]},
    {"level": "INFO", "counts": [410, 398, 405, 420]}
  ],
  "markers": [{"name": "v1.4.3", "kind": "deploy", "at": "...", "note": null}]
}
```

`counts[i]` covers `from + i * step_seconds`. Series are ordered most severe
first. With `pattern_id`, the one series has `pattern_id` and `level`.

### `GET /diagnose`

The likely causes of what is wrong now, most likely first. The same answer as
the `diagnose` tool, in full detail and without a size limit.

| Parameter | Default |
|---|---|
| `window` | since a deploy in the last 6h, else `1h` |
| `min_level` | all |
| `limit` | 5 (up to 20) |

Each cause has `pattern_id`, `kind` (as in `/whats-new`), `level`, `template`,
`why`, `started_with` (a deploy), `location`, `samples` and `request_id`; the
first also has `request`, the lines of that request. With nothing new,
`ongoing` lists the busiest warnings and errors.

### `GET /whats-new`

What is new, returning, spiking, rising or gone quiet in a recent window compared with
a baseline. Same rules as the `whats_new` tool.

| Parameter | Default |
|---|---|
| `window` | `1h` |
| `baseline` | `24h` |
| `min_level` | all |
| `limit`, `offset` | 20, 0 (limit up to 200) |

```json
{
  "window": {"from": "...", "to": "..."},
  "baseline": {"from": "...", "to": "..."},
  "summary": "1 new, 0 returning, 0 spiking, 0 gone quiet. 812 events across 9 patterns in the window.",
  "counts": {"new": 1, "returning": 0, "spike": 0, "rising": 0, "gone_quiet": 0},
  "traffic": {"events_window": 812, "events_baseline": 19003, "ratio": 1.0},
  "current_release": "v1.4.3",
  "markers": [],
  "total": 1,
  "offset": 0,
  "findings": [
    {
      "kind": "new",
      "note": null,
      "count_window": 27,
      "count_baseline": 0,
      "per_hour_window": 27.0,
      "per_hour_baseline": 0.0,
      "pattern": { "...": "a pattern, see below" },
      "sample": {"at": "...", "text": "ERROR checkout failed ...\nTraceback ..."}
    }
  ]
}
```

`kind` is `new`, `regressed`, `returning`, `spike`, `rising` or `gone_quiet`. Ignored
patterns are left out.

### Patterns

A pattern, wherever one appears:

```json
{
  "id": "a1b2c3d4e5f6",
  "level": "ERROR",
  "template": "checkout failed request_id=<REQ> | KeyError",
  "exception": "KeyError",
  "location": "shop/pricing.py:41 in price_cart",
  "first_seen": "...",
  "last_seen": "...",
  "first_release": "v1.4.3",
  "total": 27,
  "example_request": "badreq-0001",
  "state": {"state": "open", "since": null, "note": null}
}
```

`state.state` is `open`, `resolved`, `resolved_next` (resolved from the next
deploy on), `ignored` or `regressed` (resolved, then seen again). Depending on
the state it also has `resolved_at`, `regressed_at` and `regressed_in` (the
release it came back in), or `until` and `remaining` for an ignore.

#### `GET /patterns`

| Parameter | Default | |
|---|---|---|
| `since` | `24h` | Period to count over |
| `min_level` | all | |
| `contains` | | Case-insensitive text in the template |
| `limit`, `offset` | 50, 0 | limit up to 500 |
| `trend` | 24 | Points in each pattern's `trend` (0 to 96, 0 leaves it out) |
| `state` | any | Only patterns in this state |

Returns `{since, trend_step_seconds, total, offset, patterns}`, each pattern
with `count` in the period and `trend`, its counts over the period. Most severe,
then most frequent first.

#### `GET /patterns/{id}`

The pattern plus `locations` (`[{location, count}]`), `count_last_hour`,
`count_last_24h` and up to 5 `samples` (`[{at, text}]`, newest first). An ID
that was merged into another pattern resolves to it, with `merged_from` set.

With `--metric`, it also has `fields`: each field read from its lines over the
last 24 hours, as in `GET /fields`.

#### `GET /patterns/{id}/verify?since=v1.4.3`

The `verify_fix` verdict. `since` is required; `before` is optional.

```json
{
  "pattern": { "...": "..." },
  "verdict": "fixed",
  "explanation": "At the old rate about 14 would have occurred since the split; none did.",
  "split": {"at": "...", "marker": "v1.4.3"},
  "before": {"from": "...", "to": "...", "count": 27, "per_hour": 46.2},
  "after": {"from": "...", "to": "...", "count": 0, "per_hour": 0.0},
  "expected_after": 14.2,
  "confidence": "high",
  "wait_minutes": null,
  "caveat": null
}
```

`verdict` is `fixed`, `reduced`, `still_occurring`, `worse`, `too_early`,
`appeared_after` or `not_active_before`. `before` is `null` for
`appeared_after`.

### `GET /compare`

What changed around a deploy, like `compare_deploy`. `at` defaults to the
latest deploy; `before`, `limit` and `offset` are optional.

Returns `{split, before: {from, to, events}, after: {from, to, events},
summary, counts, total, offset, changes}`. Each change has `kind` (`new_after`,
`increased`, `stopped`, `decreased`), `before` and `after` (`{count,
per_hour}`) and `pattern`.

### `GET /context`

The redacted raw lines around an event. Pass `pattern_id`, `at`, or both;
`before` and `after` are line counts (default 20, at most 500).

Returns `{anchor, lines}`; each line is `{id, at, pattern_id, text}`, oldest
first, and includes the anchor.

### `GET /requests/{id}`

Every event of one request in order, from the raw buffer (at most 2,000).

```json
{
  "request_id": "badreq-0001",
  "service": null,
  "started": "...",
  "duration_ms": 41,
  "events": 4,
  "truncated": false,
  "lines": [{"id": 1, "at": "...", "offset_ms": 0, "pattern_id": "...", "text": "..."}]
}
```

#### `POST /patterns/{id}/state`

```json
{"state": "resolved", "note": "fixed by raising the retry budget"}
{"state": "ignored", "ignore_for": "24h"}
{"state": "ignored", "ignore_count": 100}
{"state": "open"}
```

Returns the pattern with its new state. A resolved pattern that occurs again
is reported as `regressed` by `/whats-new`, alerts and the pattern itself.

#### `POST /patterns/merge`

`{"ids": ["p1", "p2", "p3"]}` merges the others into the first. Their counts,
samples and locations move over, and their IDs keep resolving to it. Returns
the merged pattern.

### Releases

- `GET /releases` lists deploys newest first: `{releases: [{name, at, until, current}]}`.
- `GET /releases/{name}` (or `current`) is a release's report card:

```json
{
  "name": "v1.4.3",
  "at": "...",
  "until": null,
  "current": true,
  "previous": "v1.4.2",
  "env": "production",
  "events_per_hour": 820.5,
  "errors_per_hour": 0.4,
  "previous_errors_per_hour": 61.6,
  "new": [ { "...": "patterns first seen in this release" } ],
  "regressed": [ { "...": "...", "regressed_at": "..." } ],
  "resolved": [ { "...": "...", "resolved_at": "..." } ]
}
```

The previous release's error rate is measured over at most as long as this one
has been live, so a release an hour old isn't compared with a week.

### Fields

Numbers read from log lines with `--metric duration_ms`. They are kept per
hour, so times are read to the hour.

- `GET /fields?since=24h&pattern_id=...` summarises each field:
  `{from, to, pattern_id, fields: [{field, count, mean, p50, p95, p99, min, max}]}`,
  most used first. Percentiles are within about 9%; the rest are exact.
- `GET /fields/{name}` is one field over time. It takes `since` (default `24h`),
  `until`, `pattern_id` and `step` (whole hours, default about 100 points).

```json
{
  "field": "duration_ms",
  "pattern_id": null,
  "from": "2026-10-30T00:00:00.000Z",
  "to": "2026-11-06T12:00:00.000Z",
  "step_seconds": 86400,
  "summary": {"field": "duration_ms", "count": 3584, "mean": 132.2, "p50": 103.1, "p95": 317.9, "p99": 583.1, "min": 18, "max": 1845},
  "points": [{"at": "...", "count": 512, "mean": 113.4, "p50": 94.5, "p95": 245.1, "p99": 490.3, "max": 1059}]
}
```

A point with no values has `count` 0 and `null` for the rest.

### Markers

- `GET /markers?limit=100` returns `{markers: [{name, kind, at, note}]}`, newest first.
- `POST /markers` with `{"name": "fix-currency", "note": "...", "at": "5m", "deploy": false}`
  adds one and returns it with status 201. `deploy: true` records a release.

### Alerts

- `GET /alerts` returns `{enabled, alert}`: the latest push alert, or `null`.
- `GET /alerts/stream` is a server-sent event stream with one `alert` event per
  push alert, the same JSON as the webhook. 404 when alerts are off.

```js
const es = new EventSource("/api/v1/alerts/stream");
es.addEventListener("alert", (e) => console.log(JSON.parse(e.data)));
```

`EventSource` can't send an `Authorization` header, so with a token use
`fetch` and read the stream yourself.

## On a hub

`starlist hub` serves the same routes for every service that pushes to it.

- Add `service=NAME` to any route to ask one service. Answers then have
  `"service": "NAME"`.
- `service` can be left out when the hub has one service. It can also be left
  out for routes under `/patterns/{id}`, which find the service holding that
  pattern. Otherwise a route without it answers 400 and lists the services.
- `GET /services` returns `{services: [{name, env, current_release, patterns, last_hour}]}`.
- `GET /status` without `service` returns totals across services, with
  `"hub": true` and a `services` list.
- `GET /whats-new` without `service` merges every service's findings. Each
  finding has `service`, errors come first, and `counts` are summed.
- `GET /requests/{id}` without `service` follows the request through every
  service. Each line has `service`, and `services` gives each one's
  `events`, `first_offset_ms` and `last_offset_ms`. Order across services
  follows each host's clock.
- `/alerts/stream` answers 404; push alerts aren't available on a hub yet.

Collectors push to `POST /hub/v1/push` with their own tokens, from
`starlist hub-token SERVICE`. That route isn't part of this API.
