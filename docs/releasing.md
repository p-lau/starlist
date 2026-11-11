# Releasing

Pushing a version tag runs `.github/workflows/release.yml`, which:

1. builds the dashboard once;
2. builds `starlist` and `starlist-ci` for Linux (x86_64 and arm64, static musl), macOS (Intel and Apple silicon) and Windows (x86_64), each with the dashboard embedded;
3. publishes a GitHub release with the archives, `SHA256SUMS`, and this version's changelog section as notes;
4. pushes the container image to `ghcr.io/p-lau/starlist` as `1.2.3`, `1.2` and `1`;
5. publishes `starlist-core`, then `starlist` and `starlist-ci`, to crates.io.

## Once

- Add a `CARGO_REGISTRY_TOKEN` secret to a `crates-io` environment (Settings, Environments), with a token scoped to publishing these three crates. Requiring a reviewer on that environment gives a last check before anything reaches crates.io, which can't be undone.
- After the first image push, make the `starlist` package on GitHub public (Packages, Package settings).

## Each release

1. On a branch, set `version` in `[workspace.package]` and the `version` of the `starlist-core` dependency in `Cargo.toml` to the new version, and run `cargo check` to update `Cargo.lock`.
2. In `CHANGELOG.md`, rename `[Unreleased]` to `[1.2.3] - YYYY-MM-DD`, start a new empty `[Unreleased]` above it, and update the links at the bottom.
3. Merge it, then tag the merge commit and push the tag:

   ```sh
   git tag -a v1.2.3 -m v1.2.3
   git push origin v1.2.3
   ```

4. Check the release page, `docker run --rm ghcr.io/p-lau/starlist:1.2.3 --version` `cargo install starlist --version 1.2.3` and `cargo install starlist-ci --version 1.2.3`.

A failed job can be re-run from the Actions tab. If crates.io publishing fails after the GitHub release is out, fix the cause and re-run only the `crates` job; a version that reached crates.io can't be replaced, only yanked, so a bad one is followed by a patch release.
