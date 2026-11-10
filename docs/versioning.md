# Versioning

starlist follows [Semantic Versioning](https://semver.org/) from 1.0.0. Agents, scripts and dashboards depend on its interface, so this page says what that interface is and how it may change.

## What is stable

From 1.0.0, within a major version:

- **MCP tools** ([tools.md](tools.md)): no tool is removed or renamed, no argument is removed, renamed or made required, and no output field is removed, renamed or changes type. Enum values (`kind`, `verdict`, `state`) aren't removed or renamed.
- **JSON API** ([api.md](api.md)): `/api/v1` follows the same rules. A breaking change gets a new prefix (`/api/v2`), and the old one stays for at least one minor release.
- **Command line and `starlist.toml`**: commands, flags and environment variables aren't removed or renamed, and their defaults don't change in a way that loses data or opens access.
- **The database**: a newer release opens and upgrades an older database. Going back to an older release after an upgrade isn't supported.
- **Pattern IDs**: the same line gets the same pattern ID, so IDs an agent or a ticket holds keep working. Masking improvements that would change IDs wait for a major version.
- **Hub pushes**: the push protocol only changes in a major version, so collectors and a hub of the same major version work together, in any order of upgrade.

## What may change in a minor release

- New tools, arguments, output fields, enum values, API endpoints, flags and log sources. Clients should ignore fields they don't know.
- Tool descriptions and the MCP instructions, which are written for agents and get tuned.
- Wording inside text fields (`why`, `note`, `template` masking of new formats), ranking and thresholds of findings, and answer sizes within the documented limits.
- What redaction catches: it only ever catches more.
- The dashboard, which isn't an interface.

## starlist-core

The `starlist-core` crate is versioned with the binary. Its public Rust API follows the usual Cargo rules: a breaking change to it is a major version.

## Before 1.0.0

0.x releases may change anything, and say so in [CHANGELOG.md](../CHANGELOG.md).

## Checking it

`tests/interface.rs` starts a server, calls every tool and compares the tool names, arguments and output fields with [tools.md](tools.md). A pull request that changes them updates that file (`UPDATE_INTERFACE=1 cargo test --test interface`) and notes the change in the changelog, so a removal can't slip in unnoticed.
