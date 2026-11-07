# Running starlist in containers

The image `ghcr.io/p-lau/starlist` (Linux amd64 and arm64) holds only the static binary, CA certificates and a non-root user (uid 65532). Its working directory and volume is `/data`; the arguments are the usual command line:

```sh
docker run --rm -v app-logs:/logs:ro -v starlist:/data -p 127.0.0.1:7341:7341 \
  -e STARLIST_TOKEN=... ghcr.io/p-lau/starlist:1 \
  serve --file /logs/app.log --http 0.0.0.0:7341 --db /data/starlist.db
```

Inside a container starlist listens beyond localhost, so it needs a token: set `STARLIST_TOKEN`, or let it generate one at `/data/starlist.db.token`. Publish the port on `127.0.0.1` only, and put a TLS proxy in front to reach it from another machine.

- [compose.yaml](compose.yaml): an app and starlist sharing a log volume.
- [kubernetes/sidecar.yaml](kubernetes/sidecar.yaml): starlist as a sidecar container in the app's pod, sharing an `emptyDir`.

Both follow a log file the app writes. For an app that only logs to stdout, starlist can instead read it with `--docker NAME` or `--k8s deployment/NAME` from outside the container, or receive it over OTLP; see [docs/sources.md](../docs/sources.md).

`docker build -f deploy/Dockerfile -t starlist .` builds the image from source, dashboard included.
