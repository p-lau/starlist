# Token benchmark: raw logs vs starlist

Does an agent fix a production bug with fewer tokens when it reads logs through starlist instead of the raw log file? This benchmark runs the same debugging tasks both ways with a real coding agent and grades the fix with a hidden check.

## Setup

- **The service.** `app/` is a small checkout service in Python (about 470 lines: catalog, cart pricing, coupons, inventory, shipping quotes, payments, receipts). It has realistic background noise: slow queries, a flaky payment gateway, SMTP disconnects, a long-standing `NotFound` crash for deleted products, card declines and out-of-stock rejections.
- **The logs.** `simulate.py` runs the real service code against 26 hours of simulated traffic with a simulated clock, so every line and stack trace comes from executing the code. Version 2.3.4 runs for 25 hours, then 2.4.0 (the release with the bug) is deployed and runs for the last 45 minutes. Stock levels survive the deploy. Two sizes:
  - **small**: INFO level, about 165k lines and 17 MB.
  - **large**: DEBUG level and 8x the traffic, about 2.9M lines and 300 MB.
- **The bugs** (`scenarios.py`). Every release also carries unrelated changes (version bump, a new market), so the bug isn't the only difference.

  | Scenario | What the release broke | What shows in the logs |
  |---|---|---|
  | coupon-percent | Coupons became whole percents in config; pricing still treats them as fractions, so totals go negative | New `ValueError` from `payments.py`, under the same message as an old, unrelated crash |
  | shipping-weight | A new grams-to-kg helper divides by a million | New WARN on every quote: carrier rejects the weight, flat rate used. No errors |
  | receipt-giftcard | Gift card receipt line adds a `Decimal` to a `str` | New `TypeError` under the same message as old SMTP failures |
  | cache-config | The cache TTL setting was renamed; catalog reads the old name and gets 0 | Cache hit rate drops to 0, slow queries jump. No errors |
  | stock-boundary | Stock check became `stock > qty` | Out-of-stock rejections rise about 1.5x; the INFO line shows `requested=2 available=2` |

- **The agent.** Headless Claude Code (`claude -p`), Sonnet 5.5, default system prompt, tools limited to Read, Edit, Grep and Glob (no shell), no user settings, at most 40 turns. It gets a fresh copy of the released code with no git history and this prompt: "You're on call for the shop service... {symptom}. {how to read logs}. Find the root cause and fix it with the smallest change. You can't run the service or the tests."
- **Arms.**
  - **raw**: the log file's path. The agent reads it with Grep (which has count mode and a result limit) and Read.
  - **starlist**: the log is ingested with `starlist ingest --source-root /app`, deleted, and served with `starlist mcp --stdio`. The agent only sees starlist's tools.
  - **none** (control): no logs at all, to check whether the bug can be found from the code alone.
- **No hint.** With `--no-hint` the symptom is replaced by "An alert fired after the latest release, but nobody knows yet what is wrong", so the agent has to find the problem itself.
- **Grading.** After the agent stops, the visible tests and a hidden check (`checks/<scenario>.py`) run on a copy of its workspace. The hidden check fails on the released code and passes on a correct fix.
- **Metrics.** From the agent's stream: total tokens (input + cache reads + cache writes + output, summed over every model call), cost, turns, peak context, and the characters that log tools returned (Grep/Read on the log file, or starlist calls). "Log output tokens" is that character count divided by 4.

## Results (2026-11-06, Sonnet 5.5, 3 runs per cell, 120 runs, about $5)

Totals over the 5 scenarios × 3 runs:

| Logs | Hint | Arm | Fixed | Total tokens | Log output | Output tokens | Turns | Cost |
|---|---|---|---|---|---|---|---|---|
| small (17 MB) | symptom | raw | 15/15 | 1,100k | ≈30k | 20.7k | 127 | $0.86 |
| small (17 MB) | symptom | starlist | 15/15 | 1,006k | ≈19k | 11.5k | 100 | $0.58 |
| large (300 MB) | symptom | raw | 15/15 | 894k | ≈17k | 18.7k | 116 | $0.67 |
| large (300 MB) | symptom | starlist | 15/15 | 1,048k | ≈18k | 11.3k | 97 | $0.58 |
| large (300 MB) | none | raw | 15/15 | 981k | ≈17k | 20.5k | 132 | $0.69 |
| large (300 MB) | none | starlist | 15/15 | 1,555k | ≈22k | 17.1k | 138 | $0.75 |
| no logs | symptom | none | 15/15 | 597k | 0 | 10.6k | 81 | $0.38 |
| no logs | none | none | 15/15 | 858k | 0 | 16.0k | 138 | $0.54 |

Per-scenario medians are in `results/*.jsonl`; print them with `python3 report.py results/<file>.jsonl`.

### What this says

1. **These bugs don't need logs.** The control arm fixed all 30 with the fewest tokens. On a 470-line codebase the agent reads every file in a few turns and spots the bug, so this round can't show log tooling helping anyone find a bug. It does measure what logs cost.
2. **Raw logs don't flood the context.** Even at 300 MB, the raw-log agent's log reads were about 1k tokens per run, because it greps with counts and result limits. Log output is 1 to 3% of total tokens in either arm. A 10x cut in log tokens moves the total by a few percent at most.
3. **Most tokens are fixed overhead re-read every turn.** The system prompt and tool definitions are about 8k tokens per turn. starlist's 9 tool definitions add about 3.6k more per turn (first-turn context 11.7k vs 8.1k), which is why the starlist arm often uses more total tokens.
4. **starlist is still cheaper in dollars when the agent knows the symptom** (13 to 33% less), because the agent writes about 40% fewer output tokens and takes fewer turns. `location` and `first_release` point it at the right file, so it reasons less. Output tokens cost far more than cached input.
5. **Without a symptom, starlist did worse** (58% more tokens, 9% more cost), mostly on cache-config and stock-boundary. `whats_new` on its default 60-minute window didn't flag a 1.5x rise in out-of-stock rejections or the cache regression; agents had to try `compare_deploy`, `list_patterns` and longer windows. One agent asked `whats_new` for a 26-hour window on 26 hours of data and got "7 new" patterns and "volume 352x its baseline", which is noise.

### starlist issues found

- Fixed in this branch: Python exceptions without an `Error`/`Exception` suffix (`NotFound`, `GatewayTimeout`, `SMTPServerDisconnected`) were split from their traceback, so they lost their stack and timestamp and showed up as brand-new patterns after every ingest.
- `whats_new` with a window as long as the history compares against an empty baseline and reports everything as new and spiking.
- A deploy in the window isn't used as the comparison point automatically; a gradual rise (1.5x) or a change with no new pattern is missed unless the agent calls `compare_deploy`.
- Tool definitions cost about 3.6k tokens on every turn.
- Ingest runs at about 3.5 MB/s (300 MB in 88 s).

### Limits of this round

- One small codebase. Real services are large enough that reading every file isn't an option, which is where logs (and a code location) should matter most.
- Every bug is visible in the code. Bugs that come from data, configuration in production, an upstream change or another service can't be found without logs.
- No shell for the agent, so no `tail`/`awk` pipelines in the raw arm (Claude Code's permission rules blocked running a sub-agent with an unrestricted shell here).
- One model, three runs per cell.

## Run it

Needs Python 3.11+, a release build of starlist, and the `claude` CLI signed in.

```sh
cargo build --release
cd bench
python3 run.py --repeats 3                      # small logs, raw and starlist
python3 run.py --large --repeats 3              # 300 MB logs (each run simulates and ingests them: a few minutes)
python3 run.py --large --no-hint --repeats 3    # no symptom given
python3 run.py --arm none --repeats 3           # control: no logs
python3 report.py results/<file>.jsonl
```

Options: `--scenario NAME` (repeatable), `--arm raw|starlist|none`, `--model`, `--parallel`, `--keep` (keep logs and databases in `--work`, default `/tmp/starlist-bench`). To see a scenario's logs without an agent: `python3 simulate.py --scenario coupon-percent --out app.log`.
