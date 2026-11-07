# Builds starlist from source, dashboard included:
#   docker build -f deploy/Dockerfile -t starlist .
# Releases publish a prebuilt image instead (ghcr.io/p-lau/starlist).

FROM node:22-alpine AS dashboard
WORKDIR /src/dashboard
COPY dashboard/package.json dashboard/package-lock.json ./
RUN npm ci
COPY dashboard/ ./
RUN npm run build

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
COPY --from=dashboard /src/dashboard/build dashboard/build
RUN cargo build --release --locked && mkdir /data

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /src/target/release/starlist /usr/local/bin/starlist
COPY --from=build --chown=65532:65532 /data /data
WORKDIR /data
VOLUME /data
EXPOSE 7341
ENTRYPOINT ["/usr/local/bin/starlist"]
CMD ["--help"]
