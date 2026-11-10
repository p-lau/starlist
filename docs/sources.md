# Log sources

starlist reads logs from where you already keep them. Every source goes through the same pipeline as a tailed file: redaction, pattern clustering, merging, deploy markers, metrics and request IDs all work the same way. Sources can be combined in one collector, for example `--file app.log --docker worker`.

| Source | Flag | How it reads | Cargo feature |
|---|---|---|---|
| File | `--file PATH` | Follows the file like `tail -F` | always |
| Standard input | `--stdin` | Reads a pipe | always |
| Docker | `--docker NAME`, `--docker-label KEY=VALUE` | Runs `docker logs -f` | `docker` |
| journald | `--journald UNIT` | Runs `journalctl -f -o json` | `journald` |
| Kubernetes | `--k8s deployment/NAME`, `--k8s SELECTOR` | Runs `kubectl logs -f` per pod | `kubernetes` |
| OpenTelemetry | `--otlp ADDR` | Receives OTLP logs over HTTP | `otel` |
| Loki | `--loki URL --loki-query QUERY` | Polls `query_range` | `loki` |

All of them are on by default. Docker, journald and Kubernetes run the platform's own command-line tool, so they add no dependencies and use the access you already have (your Docker socket, your kubectl context). The `otel` feature adds a small gzip decoder; `loki` reuses the HTTP client starlist already has. To build without them, use `--no-default-features` and list the features you want.

By default every source starts with lines written from now on. `--from-start` reads what the source still has: whole files, a container's or pod's full logs, a unit's whole journal, or the last hour of a Loki stream.

## Docker

```sh
# One container, by name or ID. Restarts and re-creation are followed.
starlist serve --docker shop-api

# Every running container with a label, including ones started later
starlist serve --docker-label com.docker.compose.service=api
```

Lines are read with `--timestamps`, so when a container restarts, or `docker logs` exits for any reason, starlist starts it again from the last line it read and skips the lines it already has. A container's stdout and stderr are kept apart so their stack traces don't get mixed. Containers that appear later under a label are read from their first line.

starlist needs to be able to run `docker` (or set `STARLIST_DOCKER=podman` for Podman). To run starlist itself in a container, mount the socket and use an image with the Docker CLI:

```yaml
services:
  starlist:
    image: your-starlist-image
    command: serve --docker-label app=shop --http 0.0.0.0:7341
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - starlist:/data
    working_dir: /data
```

Anyone who can reach the Docker socket controls the host, so keep that container's MCP port behind its token (it generates one when listening beyond localhost).

## journald

```sh
starlist serve --journald api.service --journald worker.service
```

starlist runs `journalctl --follow --output=json --unit UNIT` and uses each entry's own time. journald stores every line an app writes as its own entry; a stack trace's lines are put back together into one event. An entry with a non-default priority (warning, error, critical) is given that level unless the message already starts with one; entries at the default priority keep whatever level your app wrote. The user running starlist needs to be able to read the journal, for example by being in the `systemd-journal` group. `STARLIST_JOURNALCTL` overrides the program.

## Kubernetes

```sh
# Every pod of a deployment (also statefulset/, daemonset/, replicaset/, job/)
starlist serve --k8s deployment/api --k8s-namespace shop

# Or any label selector
starlist serve --k8s 'app=api,tier=web'
```

starlist looks up the workload's selector, lists its pods every 5 seconds and runs `kubectl logs --follow --timestamps --all-containers --prefix` for each. Each container is read as its own stream, so a sidecar's lines don't land in the middle of the app's stack traces. New pods from a rollout, a scale-up or a rescheduling are read from their first line. When a container restarts, its pod is followed again from where it left off, so a crash loop's last lines and the new container's first lines both arrive. Pods that are gone are dropped.

It uses your current kubectl context and needs `get` and `list` on pods and the workload, and `get` on `pods/log`. Run in the cluster, give its service account a Role like this:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata: { name: starlist, namespace: shop }
rules:
  - apiGroups: [""]
    resources: [pods, pods/log]
    verbs: [get, list]
  - apiGroups: [apps]
    resources: [deployments, statefulsets, daemonsets, replicasets]
    verbs: [get]
```

`STARLIST_KUBECTL` overrides the program.

## OpenTelemetry (OTLP)

```sh
starlist serve --otlp 127.0.0.1:4318
```

starlist then accepts OTLP logs at `http://127.0.0.1:4318/v1/logs`, the OTLP/HTTP endpoint, in protobuf or JSON, optionally gzipped. Point an SDK's log exporter at it:

```sh
OTEL_LOGS_EXPORTER=otlp \
OTEL_EXPORTER_OTLP_LOGS_PROTOCOL=http/protobuf \
OTEL_EXPORTER_OTLP_LOGS_ENDPOINT=http://127.0.0.1:4318/v1/logs \
  python -m myapp
```

Or send a copy from an OpenTelemetry Collector you already run:

```yaml
exporters:
  otlphttp/starlist:
    logs_endpoint: http://starlist:4318/v1/logs
    headers: { Authorization: "Bearer ${env:STARLIST_OTLP_TOKEN}" }
service:
  pipelines:
    logs:
      exporters: [otlphttp/starlist]
```

Each log record becomes one event. Its severity becomes the level, its body the message, and its trace and span IDs are kept, so `get_request` with a trace ID returns every record of that trace. The `exception.type`, `exception.message` and `exception.stacktrace` attributes become the event's stack trace, which gives the pattern its exception type and code location. Other attributes, including the resource's (like `service.name`), stay on the event, so `--metric` and `--request-key` can read them.

Listening beyond localhost needs `--otlp-token TOKEN` (or `STARLIST_OTLP_TOKEN`, at least 16 characters), sent as `Authorization: Bearer TOKEN`. The endpoint only takes `application/x-protobuf` and `application/json`, which a web page on another site can't send to it. gRPC (port 4317) isn't supported; use OTLP/HTTP.

## Loki

```sh
starlist serve --loki http://loki:3100 --loki-query '{app="api", env="prod"}'

# Grafana Cloud: user ID and an access token, as basic auth
starlist serve --loki https://logs-prod-012.grafana.net --loki-user 123456 \
  --loki-token "$GRAFANA_TOKEN" --loki-query '{service_name="api"}'
```

starlist asks `query_range` every 2 seconds for the lines since the last ones it read, looking back 15 seconds more so lines that reach Loki late aren't missed, and skipping lines it already has. The query has to be a log query (a selector, optionally with filters like `|= "error"`), not a metric query. Lines from different streams are grouped into events separately, so two pods' stack traces don't mix. Streams are told apart by their own labels only, not by labels Loki adds per line (like Loki 3's `detected_level`) or labels the query parses, so a stack trace stays in one stream. `--loki-token` alone is sent as a bearer token; `--loki-org` sets `X-Scope-OrgID` for a multi-tenant Loki.

If Loki already holds your logs, this lets an agent get starlist's patterns, `whats_new` and `verify_fix` over them without changing how they're shipped.
