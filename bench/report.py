"""Summarize results files as a Markdown table.

    python3 report.py results/v1-small-sonnet-5-5.jsonl
    python3 report.py results/v1-large-no-hint-*.jsonl   # compare runs side by side

With one file, runs are grouped by scenario and arm. With several, each file's runs stay
apart and are labeled by file name, so the same arm from two runs can be compared.
"""

import json
import sys
from collections import defaultdict
from pathlib import Path
from statistics import median


def fmt_k(n: float) -> str:
    return f"{n / 1000:.1f}k" if n >= 1000 else f"{n:.0f}"


def main() -> None:
    paths = sys.argv[1:]
    if not paths:
        sys.exit(__doc__)
    many = len(paths) > 1
    rows = []
    for path in paths:
        for line in open(path):
            if line.strip():
                r = json.loads(line)
                r["_label"] = f"{r['arm']} ({Path(path).stem})" if many else r["arm"]
                rows.append(r)
    labels = list(dict.fromkeys(r["_label"] for r in rows))
    groups = defaultdict(list)
    for r in rows:
        groups[(r["scenario"], r["_label"])].append(r)
    models = sorted({r["model"] for r in rows})
    suites = sorted({r.get("suite", "?") for r in rows})
    print(f"Suite: {', '.join(suites)}. Model: {', '.join(models)}. {len(rows)} runs. "
          "Medians per scenario.\n")
    print("| Scenario | Arm | Fixed | Total tokens | Log output (≈tokens) | Peak context | Turns | Cost |")
    print("|---|---|---|---|---|---|---|---|")
    for scenario in sorted({k[0] for k in groups}):
        for label in labels:
            g = groups.get((scenario, label))
            if not g:
                continue
            fixed = sum(r["fixed"] for r in g)
            print(f"| {scenario} | {label} | {fixed}/{len(g)} "
                  f"| {fmt_k(median(r['total_tokens'] for r in g))} "
                  f"| {fmt_k(median(r['log_output_chars'] / 4 for r in g))} "
                  f"| {fmt_k(median(r['peak_context'] for r in g))} "
                  f"| {median(r['turns'] or 0 for r in g):.0f} "
                  f"| ${median(r['cost_usd'] or 0 for r in g):.2f} |")
    print()
    for label in labels:
        g = [r for r in rows if r["_label"] == label]
        print(f"- **{label}**: fixed {sum(r['fixed'] for r in g)}/{len(g)}, "
              f"total tokens {fmt_k(sum(r['total_tokens'] for r in g))}, "
              f"log output ≈{fmt_k(sum(r['log_output_chars'] for r in g) / 4)} tokens, "
              f"cost ${sum(r['cost_usd'] or 0 for r in g):.2f}")
    raw = [r for r in rows if r["arm"] == "raw"]
    sl = [r for r in rows if r["arm"] == "starlist"]
    if raw and sl and not many:
        def ratio(key):
            a, b = sum(r[key] for r in raw), sum(r[key] for r in sl)
            return a / b if b else float("inf")
        print(f"\nRaw / starlist: total tokens {ratio('total_tokens'):.2f}x, "
              f"log output {ratio('log_output_chars'):.2f}x, "
              f"cost {sum(r['cost_usd'] or 0 for r in raw) / max(sum(r['cost_usd'] or 0 for r in sl), 1e-9):.2f}x.")


if __name__ == "__main__":
    main()
