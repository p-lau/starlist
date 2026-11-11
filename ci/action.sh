#!/usr/bin/env bash
# Run by action.yml when the job failed: get starlist-ci, find the last passing
# run's log, compare, and write the report to the job summary (and, if asked,
# the pull request). Problems here are reported as warnings and never fail the
# job: the job has already failed for its own reason.
set -uo pipefail

warn() { echo "::warning title=starlist-ci::$*"; }

if [ ! -f "$LOG" ]; then
  warn "no log at $LOG to compare"
  exit 0
fi

tmp="$RUNNER_TEMP/starlist-ci"
rm -rf "$tmp"
mkdir -p "$tmp/bin" "$tmp/baseline"

# 1. The binary: one already on PATH, a release archive, or cargo.
exe="$tmp/bin/starlist-ci"
case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64) target=x86_64-unknown-linux-musl ;;
  Linux-ARM64) target=aarch64-unknown-linux-musl ;;
  macOS-X64) target=x86_64-apple-darwin ;;
  macOS-ARM64) target=aarch64-apple-darwin ;;
  Windows-X64) target=x86_64-pc-windows-msvc ;;
  *) target="" ;;
esac
# The action's own tag (like v1.1.0) picks the matching binary.
if [ -z "$VERSION" ]; then
  if [[ "${ACTION_REF:-}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then VERSION="$ACTION_REF"; else VERSION=latest; fi
fi
if [ "$VERSION" = latest ]; then
  base="https://github.com/p-lau/starlist/releases/latest/download"
else
  base="https://github.com/p-lau/starlist/releases/download/v${VERSION#v}"
fi
installed=""
if command -v starlist-ci > /dev/null; then
  exe=starlist-ci
  installed=1
elif [ -n "$target" ]; then
  if [ "$RUNNER_OS" = Windows ]; then
    if curl -fsSL "$base/starlist-ci-$target.zip" -o "$tmp/ci.zip" \
      && 7z x -y -o"$tmp/unpack" "$tmp/ci.zip" > /dev/null; then
      cp "$tmp/unpack/starlist-ci-$target/starlist-ci.exe" "$tmp/bin/" && exe="$tmp/bin/starlist-ci.exe" && installed=1
    fi
  elif curl -fsSL "$base/starlist-ci-$target.tar.gz" -o "$tmp/ci.tar.gz" \
    && tar xzf "$tmp/ci.tar.gz" -C "$tmp/bin" --strip-components 1; then
    installed=1
  fi
fi
if [ -z "$installed" ]; then
  echo "No starlist-ci release archive for this runner; installing with cargo."
  args=(install starlist-ci --locked --root "$tmp/cargo")
  [ "$VERSION" != latest ] && args+=(--version "${VERSION#v}")
  if cargo "${args[@]}"; then
    exe="$tmp/cargo/bin/starlist-ci"
  else
    warn "could not install starlist-ci"
    exit 0
  fi
fi

# 2. The baseline: this log from the last passing run of the same workflow.
workflow="${GITHUB_WORKFLOW_REF%@*}"
workflow="${workflow##*/}"
branch="${BASELINE_BRANCH:-${GITHUB_BASE_REF:-$GITHUB_REF_NAME}}"
baseline=""
runs=$(gh run list -R "$GITHUB_REPOSITORY" --workflow "$workflow" --branch "$branch" \
  --status success --limit 10 --json databaseId -q '.[].databaseId' 2> "$tmp/gh.err")
[ -s "$tmp/gh.err" ] && warn "could not list runs: $(head -c 300 "$tmp/gh.err")"
for id in $runs; do
  if gh run download "$id" -R "$GITHUB_REPOSITORY" -n "$NAME" -D "$tmp/baseline/$id" 2> /dev/null; then
    baseline=$(find "$tmp/baseline/$id" -type f | head -n 1)
    [ -n "$baseline" ] && echo "Baseline: run $id on $branch" && break
  fi
done
[ -z "$baseline" ] && echo "No passing run on $branch with a $NAME artifact; listing errors and warnings instead."

# 3. The report.
report="$tmp/report.md"
args=(--format markdown "$LOG")
[ -n "$baseline" ] && args=(--baseline "$baseline" "${args[@]}")
if ! "$exe" "${args[@]}" > "$report"; then
  warn "starlist-ci failed"
  exit 0
fi
cat "$report" >> "$GITHUB_STEP_SUMMARY"
echo "report=$report" >> "$GITHUB_OUTPUT"

# 4. The pull request comment, one per artifact name, updated in place.
if [ "$COMMENT" = true ] && [ -n "$PR" ]; then
  marker="<!-- starlist-ci:$NAME -->"
  body="$marker
$(cat "$report")"
  id=$(gh api "repos/$GITHUB_REPOSITORY/issues/$PR/comments" --paginate \
    -q ".[] | select(.body | startswith(\"$marker\")) | .id" 2> /dev/null | tail -n 1)
  if [ -n "$id" ]; then
    gh api -X PATCH "repos/$GITHUB_REPOSITORY/issues/comments/$id" -f body="$body" > /dev/null \
      || warn "could not update the pull request comment"
  else
    gh api -X POST "repos/$GITHUB_REPOSITORY/issues/$PR/comments" -f body="$body" > /dev/null \
      || warn "could not comment on the pull request"
  fi
fi
exit 0
