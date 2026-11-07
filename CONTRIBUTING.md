# Contributing to starlist

Thanks for helping. Bug reports, fixes, new log formats and redaction rules are all welcome. For a larger change, such as a new tool, a new log source or a change to tool output, please open an issue first so we can agree on the shape before you build it. Tool names and output fields are part of the public interface that agents rely on.

Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Layout

- `src/`: the `starlist` binary (MCP server, storage, API, hub, log sources).
- `core/`: the `starlist-core` crate (parsing, pattern IDs and merging, redaction, request IDs, code locations), with no storage or server.
- `dashboard/`: the Svelte dashboard embedded in release builds.
- `site/`: the project website.
- `bench/`: the agent benchmark (Python), described in `bench/README.md`.
- `docs/`: the API reference and design notes.

## Building and testing

You need stable Rust. Node is only needed for the dashboard (version in `dashboard/.nvmrc`); without a built dashboard, the binary serves a page saying how to build it.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# The plain collector, without the dashboard, hub, Prometheus and extra sources
cargo test --workspace --locked --no-default-features
```

CI runs these on Linux, macOS and Windows, and builds and checks the dashboard on Linux:

```sh
cd dashboard && npm ci && npm run check && npm run build
```

To try a change against a realistic log, the demo in the README generates 26 hours of a fake service with a bad deploy and a fix.

## Pull requests

- Keep each PR to one change, and add or update tests for it. Redaction changes need a test with the value that should (or shouldn't) be caught.
- Update the README or `docs/api.md` when behaviour, flags or output change.
- Make sure the commands above pass. New code should build both with default features and with `--no-default-features`.
- By contributing, you agree that your work is licensed under the [MIT License](LICENSE).
