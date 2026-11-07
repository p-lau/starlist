# Following a request across services

Status: built in v0.12.0 (`starlist hub`). This note keeps the reasoning; the
README describes how to run it.

## What a single collector does

- Each event's request ID is read before masking, from a W3C `traceparent`, the
  usual `trace_id` / `request_id` / `x-request-id` / `correlation_id` fields, or
  extra keys given with `--request-key`. The ID is replaced by `<REQ>` in the
  pattern template, so requests never split a pattern.
- The ID is stored with the event in the raw buffer. `get_request` reads it back
  in time order. Findings carry `example_request`, the latest request that
  logged the pattern.
- An ID that redaction would change is dropped, so a token in an ID field is
  never kept as a request ID.

## With the hub

Collectors already push distilled events to the hub. For requests they would
also push, per event: the request ID, the service name, the timestamp, the
pattern ID, and the redacted line (the same text the raw buffer holds). The
hub keeps its own bounded buffer keyed by request ID.

`get_request` on the hub then returns one timeline across services:

```
0s     [api]      GET /checkout
+12ms  [pricing]  pricing cart in EUR
+30ms  [pricing]  ERROR KeyError: 'currency'   (pricing.py:41)
+41ms  [api]      500 Internal Server Error
```

How each point was settled:

1. **Which ID joins services.** Only IDs that propagate across calls join a
   request: a W3C trace ID does, a per-service request ID usually doesn't. The
   hub should prefer the trace ID when an event has both, which means collectors
   should keep both rather than the first one found. *Done:* when an event has
   several IDs, the earliest default key wins, and `trace_id` comes first.
2. **Clock skew.** Order by timestamp within a service, and by timestamp
   across services only to the precision the hosts' clocks allow. Showing
   per-service offsets avoids implying an order the clocks can't support.
   *Done:* each service's first and last offset is shown, with a note that
   order across hosts follows their clocks.
3. **Sampling and size.** A busy service can log thousands of events per
   request. The hub applies the same `max_chars` budget, keeping each service's
   first and last events and its errors before anything from the middle.
   *Done.*
4. **Privacy.** The hub's buffer is raw text from every service, so it gets the
   same `--raw-size` limit and `--no-raw` switch as a collector, and collectors
   with `--no-raw` send no lines to it. *Done:* `--raw-size` applies per service.
