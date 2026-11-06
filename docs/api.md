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
- `service` is reserved for the hub and is `null` on a single collector.

Until 1.0.0 fields may still change. From 1.0.0, `/api/v1` only gains fields.

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
  "alerts": true
}
```

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

### `GET /whats-new`

What is new, returning, spiking or gone quiet in a recent window compared with
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
  "counts": {"new": 1, "returning": 0, "spike": 0, "gone_quiet": 0},
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

`kind` is `new`, `regressed`, `returning`, `spike` or `gone_quiet`. Ignored
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
