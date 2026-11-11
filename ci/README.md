# starlist-ci

When CI goes red, the job log explains why, but it is long and most of it is the same in every run. `starlist-ci` compares the failing run's log with the last passing run's and reports only what changed: patterns that are new, patterns that grew, and patterns that stopped, each with a sample line, its line number, its step and, for a stack trace, the code location. The answer is a few hundred tokens for an agent, or a Markdown table for people.

It uses the same engine as [starlist](https://github.com/p-lau/starlist) (`starlist-core`): lines are grouped into events with their stack traces, secrets are redacted, and values like numbers, IDs and durations are masked so that `retrying request 1 of 3` and `retrying request 2 of 3` count as one pattern. It runs once and exits: no server, no database, nothing written to disk.

## Install

```sh
cargo install starlist-ci
```

Release archives for Linux, macOS and Windows are on the [releases page](https://github.com/p-lau/starlist/releases) as `starlist-ci-<target>`.

## Use

```sh
# Save each run's output
cargo test 2>&1 | tee test.log

# After a failure, against the last passing run's log
starlist-ci --baseline passing.log failing.log
starlist-ci --baseline passing.log --format markdown < failing.log

# No passing log yet: list the failing log's errors and warnings
starlist-ci failing.log
```

It reads plain output, `gh run view --log` output (the step name comes from its columns) and raw job logs downloaded from GitHub Actions (runner timestamps, color codes and `##[group]` lines are cleaned up, and `##[error]` lines count as errors).

| Option | |
|---|---|
| `-b, --baseline FILE` | The last passing run's log. Without it, errors and warnings are listed instead. |
| `--format json\|markdown` | `json` (default) for agents, `markdown` for a job summary or a pull request comment. |
| `--detail brief\|full` | `full` gives every finding a sample and longer templates. |
| `--limit N` | Most findings to list (10 brief, 30 full). |
| `--max-chars N` | Answer size limit, as for starlist's MCP tools (8000 brief, 16000 full, at least 1000). Samples are shortened, then dropped, then findings left out, and the answer says so. |
| `--min-level LEVEL` | Leave out findings below `DEBUG`, `INFO`, `WARN`, `ERROR` or `FATAL`. Default: none with a baseline, `WARN` without. |
| `--source-root DIR` | Strip this prefix from code locations; repeatable. `$GITHUB_WORKSPACE` by default. |
| `--redact REGEX`, `--redact-ips` | Redact more than the built-in rules. |
| `--merge-after N` | Merge patterns that differ in one word once it takes N values (default 5; 0 turns it off). |

## Output

```json
{
  "summary": "3 new, 1 grew, 2 stopped compared with the passing run. Start with line 8: ERROR payment failed for order <NUM> | ValueError",
  "events": {"failing": 9, "passing": 6},
  "patterns": {"failing": 7, "passing": 6},
  "findings": [
    {
      "kind": "new",
      "pattern_id": "pfe9f0002fe",
      "level": "ERROR",
      "template": "ERROR payment failed for order <NUM> | ValueError",
      "count": 1,
      "before": 0,
      "location": "billing/charge.py:88 in charge",
      "step": "Run tests",
      "line": 8,
      "sample": "ERROR payment failed for order 42\nTraceback (most recent call last):\n  ..."
    },
    {"kind": "grew", "level": "WARN", "template": "WARN retrying request <NUM> of <NUM>", "count": 3, "before": 1, "line": 4},
    {"kind": "stopped", "level": "INFO", "template": "INFO seeding fixtures done", "count": 0, "before": 1, "passing_line": 6}
  ]
}
```

- `kind`: `new` (only in the failing run), `grew` (at least twice as often, and at least two more), or `stopped` (only in the passing run). Without a baseline there is no `kind` or `before`.
- `count` and `before`: events in the failing and the passing run.
- `line`: where the pattern first appears in the failing log, so an agent can read around it. A pattern that stopped has `passing_line` instead.
- `level`: the logged level, or `ERROR?` for a line with no level that reads as a failure (`error:`, `FAILED`, `panicked at`, `exit code 1`, ...). Those are ranked with errors.
- Findings are ordered by level, then new before grew before stopped, then by line, so the first error of the run comes first. In brief detail only the first three carry a sample, and a sample that would only repeat the template is left out.

A pattern ID is stable within one comparison, not across comparisons.

## GitHub Action

The action in this folder keeps each run's log as an artifact and, when the job fails, compares it with the log of the last passing run of the same workflow, on the pull request's base branch (or the current branch). The report goes to the job summary and, if you ask, to a pull request comment that is updated on later failures. It never fails the job itself.

```yaml
permissions:
  contents: read
  actions: read          # find the last passing run and download its log
  pull-requests: write   # only with comment: true

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Tests
        shell: bash      # with pipefail, so tee keeps the test's exit code
        run: cargo test 2>&1 | tee test.log
      - uses: p-lau/starlist/ci@v1.1.0   # a release tag
        if: always()
        with:
          log: test.log
          status: ${{ job.status }}
          comment: true
```

| Input | |
|---|---|
| `log` | The log file the job wrote. Required. |
| `status` | `${{ job.status }}`. Required. The log is kept on every run and compared only on `failure`. |
| `name` | Artifact name, `starlist-log-<job id>` by default. Give each matrix entry its own, like `starlist-log-${{ matrix.os }}`. |
| `baseline-branch` | Branch whose last passing run is the baseline. |
| `comment` | `true` to post the report on the pull request. |
| `version` | starlist-ci version. By default the one matching the action's tag, or the latest release. |
| `retention-days` | How long to keep each log artifact (14 days by default). |

The output `report` is the path of the Markdown report. The action uses a `starlist-ci` already on `PATH`, otherwise the release archive for the runner, otherwise `cargo install`.

## For agents

An agent fixing a red build can run it itself:

```sh
gh run view <failed-run-id> --log > failing.log
gh run view <last-passing-run-id> --log > passing.log
starlist-ci --baseline passing.log failing.log
```

Then read the failing log around the reported `line` numbers, instead of the whole thing.
