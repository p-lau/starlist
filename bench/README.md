# Token benchmark: raw logs vs starlist

Does an agent fix a production bug with fewer tokens when it reads logs through starlist instead of the raw log file? This benchmark runs the same debugging tasks both ways with a real coding agent and grades each fix with a check the agent never sees.

**Short answer (Sonnet 5.5, 255 runs, about $12):** it depends on whether the bug needs the logs at all.

- **When the code alone gives the bug away** (v1), starlist makes no real difference. The agent finds the bug by reading the code, and log reads are 1 to 3% of its tokens.
- **When only the logs can explain the failure** (v2), starlist fixes as many bugs as raw logs with 32% fewer tokens and half the cost on a 17 MB log. On a 300 MB log it takes 2.7x fewer tokens and 2.9x less cost. Without logs the agent fixed 2 of 12.

## Setup

- **The service.** A small checkout service in Python (about 470 lines: catalog, pricing, coupons, inventory, shipping quotes, payments, receipts). It has realistic background noise: slow queries, a flaky payment gateway, mail relay disconnects, a long-standing crash for deleted products, card declines and out-of-stock rejections.
- **The logs.** The simulator runs the real service code against 26 hours of simulated traffic with a simulated clock, so every line and stack trace comes from executing the code. There are two sizes:
  - **small**: INFO level, about 170k lines and 17 MB.
  - **large**: DEBUG level and 8x the traffic, about 2.9M lines and 300 MB.
- **The agent.** Headless Claude Code (`claude -p`) on Sonnet 5.5, with the default system prompt and no user settings, at most 40 turns. Its tools are limited to Read, Edit, Grep and Glob, with no shell. It gets a fresh copy of the code with no git history and this prompt: "You're on call for the shop service... {symptom}. {how to read logs}. Find the root cause and fix it with the smallest change. You can't run the service or the tests."
- **Arms.**
  - **raw**: the agent gets the log file's path and reads it with Grep (which has a count mode and a result limit) and Read.
  - **starlist**: the log is ingested with `starlist ingest --source-root /app`, deleted, and served with `starlist mcp --stdio`. The agent sees only starlist's tools.
  - **none** (control): no logs at all, to check whether the bug can be found from the code alone.
- **Grading.** When the agent stops, the visible tests and a hidden check run on a copy of its workspace. Each hidden check fails on the code as shipped and passes on a correct fix.
- **Metrics** (from the agent's stream):
  - total tokens: input, cache reads, cache writes and output, summed over every model call
  - cost and turns
  - peak context
  - log output: the characters returned by log tools (Grep or Read on the log file, or starlist calls), divided by 4 to estimate tokens

### v1: bugs a release introduced (`v1/`)

Version 2.3.4 runs for 25 hours, then 2.4.0 is deployed and runs for the last 45 minutes. The release also carries unrelated changes, so the bug isn't the only difference.

| Scenario | What the release broke | What shows in the logs |
|---|---|---|
| coupon-percent | Coupons became whole percents in config; pricing still treats them as fractions, so totals go negative | New `ValueError` from `payments.py`, under the same message as an old, unrelated crash |
| shipping-weight | A new grams-to-kg helper divides by a million | New WARN on every quote: the carrier rejects the weight and the flat rate is used. No errors |
| receipt-giftcard | The gift card receipt line adds a `Decimal` to a `str` | New `TypeError` under the same message as old SMTP failures |
| cache-config | The cache TTL setting was renamed; catalog reads the old name and gets 0 | Cache hit rate drops to 0 and slow queries jump. No errors |
| stock-boundary | The stock check became `stock > qty` | Only the INFO line `requested=2 available=2`; out-of-stock rejections rise late in the day in every scenario, so their rate says nothing |

`--no-hint` replaces the symptom with "An alert fired after the latest release, but nobody knows yet what is wrong."

### v2: bugs only the logs reveal (`v2/`)

Nothing is deployed. About 40 minutes before the end of the log, something outside the service changes, and code that looks fine starts failing. The database and the upstream services are simulated outside the service code (`v2/simulate.py`), so reading the code shows a normal HTTP client and nothing about what changed. The symptom always says "Nothing was deployed today."

| Scenario | What changed outside the service | Symptom given |
|---|---|---|
| gateway-rate-limit | The payment gateway answers 18% of charges with code `rate_limited`; the service treats unknown codes as declines | More customers are told their card was declined |
| carrier-format | The carrier's rate API moved the price to `rate.amount_cents` | Shipping prices look wrong |
| product-data | A catalog import left five popular products with no weight | Some checkouts fail with a server error |
| mobile-qty | A new iOS app release sends quantities as strings | Some checkouts fail with a server error (same symptom as product-data) |

## Results

All runs used Sonnet 5.5 with 3 runs per scenario. The table shows totals over each suite's scenarios. "Fixed" means the hidden check passed.

| Suite | Logs | Arm | Fixed | Total tokens | Log output | Output tokens | Turns | Cost |
|---|---|---|---|---|---|---|---|---|
| v1 | small, symptom | raw | 15/15 | 1,100k | ≈30k | 20.7k | 127 | $0.86 |
| v1 | small, symptom | starlist | 15/15 | 1,006k | ≈19k | 11.5k | 100 | $0.58 |
| v1 | small, symptom | starlist after fixes | 15/15 | 928k | ≈8k | 9.9k | 90 | $0.68 |
| v1 | small, symptom | none | 15/15 | 597k | 0 | 10.6k | 81 | $0.38 |
| v1 | large, symptom | raw | 15/15 | 894k | ≈17k | 18.7k | 116 | $0.67 |
| v1 | large, symptom | starlist | 15/15 | 1,048k | ≈18k | 11.3k | 97 | $0.58 |
| v1 | large, no symptom | raw | 15/15 | 981k | ≈17k | 20.5k | 132 | $0.69 |
| v1 | large, no symptom | starlist | 15/15 | 1,555k | ≈22k | 17.1k | 138 | $0.75 |
| v1 | large, no symptom | starlist after fixes | 15/15 | 1,323k | ≈12k | 13.3k | 117 | $0.67 |
| v1 | none, no symptom | none | 15/15 | 858k | 0 | 16.0k | 138 | $0.54 |
| v1 | large, no symptom | raw, re-run | 15/15 | 1,068k | ≈18k | 22.0k | 140 | $0.75 |
| v1 | large, no symptom | starlist with `diagnose` | 15/15 | 1,192k | ≈11k | 12.6k | 101 | $0.69 |
| v1 | large, no symptom | starlist with `diagnose`, `--tools core` | 15/15 | 1,121k | ≈11k | 13.5k | 105 | $0.62 |
| v2 | small | raw | 12/12 | 1,416k | ≈47k | 23.4k | 141 | $0.98 |
| v2 | small | starlist | 12/12 | 960k | ≈8k | 12.2k | 95 | $0.47 |
| v2 | small | none | **2/12** | 1,089k | 0 | 25.8k | 164 | $0.74 |
| v2 | large | raw | 12/12 | 2,636k | ≈52k | 39.0k | 201 | $1.39 |
| v2 | large | starlist | 12/12 | 978k | ≈9k | 12.9k | 98 | $0.48 |

v2 used starlist after the fixes described below. For per-scenario medians, run `python3 report.py results/<file>.jsonl`; pass several files to compare them.

### What this says

1. **A benchmark has to need the logs.** In v1 the no-logs control fixed all 15 with the fewest tokens: on a 470-line codebase the agent reads every file and spots the bug. In v2 it fixed 2 of 12, and both of those were guesses that rate-limit codes should be retried.
2. **When logs matter, starlist saves tokens, and more as logs grow.** In v2, raw-log agents spent 1.5x the tokens on a 17 MB log and 2.7x on a 300 MB log. The biggest gap was product-data on the large log: 550k tokens and 34 turns for the median raw run, against 112k and 11 for starlist. The raw agent had to grep its way from a `TypeError` to which products were affected, while starlist's `get_request` and samples showed it in a few calls.
3. **Raw logs don't flood the context; they cost turns.** Grep with counts and limits keeps each read small: about 4k tokens of log per run even at 300 MB. The cost comes from extra turns, because each turn re-reads the whole conversation, and from two to three times the output tokens spent reasoning about what the greps showed.
4. **Fixed overhead matters.** The system prompt and tool definitions are re-read on every turn. Before the fixes, starlist's tools added 3.6k tokens per turn in this branch's version (4.6k with the full v0.12 tool set), which wiped out its savings in v1.

### What changed in starlist because of this

The benchmark found these issues, all fixed on the `starlist-fixes` branch:

- **Python exceptions not named `...Error`** (`NotFound`, `GatewayTimeout`) were split from their traceback. They lost their stack and timestamp and showed up as new patterns after every ingest.
- **Tool definitions were large.** They are now 2.4k tokens per turn instead of 4.6k: no `$schema`, plain types, terse descriptions, and the options every tool shares are explained once.
- **`whats_new` used a fixed last hour.** It now compares everything since the latest deploy when one went out in the last 6 hours.
- **The baseline counted hours with no logs.** It now starts at the first log line, so a window as long as the history no longer reports everything as new and "volume 352x its baseline".

After the fixes, starlist's v1 runs used 8 to 15% fewer tokens. The no-symptom v1 case was still worse than raw logs (1,323k vs 981k tokens).

### The no-symptom case, after `diagnose`

The `diagnose` branch adds one call that returns the likely causes with their deploy, code location, samples and the lines of a request, plus a slow-rise test and `--tools core`. The last three v1 rows of the table re-ran v1 on the large log with no symptom, raw logs included, since the agent had changed since the first run.

- **One call is usually enough.** In 12 of 15 runs the agent called only `diagnose`, then read and fixed the code. Turns dropped from 140 to about 105 and output tokens by 40%.
- **Cost is now lower than raw logs, tokens about even.** With every tool, 1,192k tokens and $0.69 against raw's 1,068k and $0.75; with `--tools core`, 1,121k and $0.62. Without stock-boundary, `--tools core` used 19% fewer tokens than raw (710k vs 880k) and 35% less cost.
- **stock-boundary has no signal in the logs.** Re-checking it showed the "1.5x rise" was not caused by the bug: out-of-stock rejections climb by the same amount late in the day in every scenario. The only trace of the bug is `requested=2 available=2` inside an INFO line. A rise test that ignores this natural swing (see `diagnose.rs`) correctly stays quiet, and the agent has to search, which costs starlist about 2.2x raw here (411k vs 188k). Per-turn cost is the reason: with no answer from the logs, every extra turn pays for starlist's tool definitions.
- **The per-turn cost of the tools matters most when logs don't help.** Peak context was about 2k tokens higher with every tool offered; `--tools core` halves the definitions.

### Limits

- One small codebase. On a large one, the v1 control would no longer read every file, and the location starlist attaches to each pattern should matter more.
- The agent had no shell, so the raw arm had no `tail`/`awk` pipelines. Claude Code's permission rules blocked running a sub-agent with an unrestricted shell here.
- One model, three runs per cell. Single runs vary a lot: raw product-data runs took between 540k and 640k tokens on the large log, and between 130k and 360k on the small one.
- In v2, the carrier-format check first also required the old rate format to keep working. Every agent that had logs diagnosed the change correctly and switched to the new format, which is a reasonable fix for an API that fully moved. That requirement was dropped and those runs were regraded. The results files mark them `regraded`.

## Layout

- `v1/` and `v2/`: one suite each, with the service (`app/`), the scenarios (`scenarios.py`), the log simulator (`simulate.py`) and the hidden checks (`checks/`). v2's simulator reuses v1's traffic and formatting.
- `results/`: every run of every suite, one JSON line per run, named `<suite>-<variant>-<model>.jsonl`. Each line has `suite`, so files from different suites can be compared or concatenated.
- `run.py` runs a suite and `report.py` summarizes any results file.

## Run it

You need Python 3.11+, a release build of starlist and the `claude` CLI signed in.

```sh
cargo build --release
cd bench
python3 run.py --repeats 3                                  # v1, small logs, raw and starlist
python3 run.py --large --no-hint --repeats 3                # v1, 300 MB logs, no symptom
python3 run.py --arm none --repeats 3                       # v1 control: no logs
python3 run.py --suite v2 --arm raw --arm starlist --arm none --repeats 3
python3 run.py --suite v2 --large --repeats 3           # a few minutes per run: 300 MB simulated and ingested
python3 run.py --large --no-hint --arm starlist --tools core --repeats 3   # a smaller tool set
python3 report.py results/<file>.jsonl                    # several files are compared side by side
```

Options:
- `--scenario NAME` (repeatable)
- `--arm raw|starlist|none`
- `--model`
- `--parallel`
- `--starlist PATH` (another build)
- `--tools all|core|NAMES` (passed to `starlist mcp`)
- `--keep` (keep logs and databases in `--work`, default `/tmp/starlist-bench`)

To see a scenario's logs without an agent, run `python3 v1/simulate.py --scenario coupon-percent --out app.log` (or `v2/simulate.py`).
