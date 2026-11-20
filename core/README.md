# starlist-core

The log engine behind [starlist](https://github.com/p-lau/starlist), a collector that watches an app's logs and serves distilled, redacted summaries to AI agents over MCP.

This crate has no storage, async runtime or network. It turns raw log text into events and leaves the rest to the collector:

- `pattern`: parses plain and JSON lines, including multi-line stack traces, into events with a stable pattern ID.
- `redact`: strips secrets and personal data before anything is stored.
- `merge`: merges patterns that differ in only one word.
- `request`: finds request and trace IDs, to follow one request's events.
- `location`: finds the app's own code location in a stack trace.

starlist-core is versioned with starlist and follows semver: a breaking change to its public API is a major version. See [docs/versioning.md](https://github.com/p-lau/starlist/blob/main/docs/versioning.md).

Most people want the `starlist` binary instead: `cargo install --locked starlist`.

## License

MIT
