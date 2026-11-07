"""Run the benchmark: each scenario, with raw logs and with starlist, a few times each.

Every run builds a fresh workspace with the released (buggy) code, simulates 26 hours of
logs ending now, starts a headless Claude Code agent with one way of reading those logs,
then grades the agent's edit with a check it never saw.

    python3 run.py --repeats 3 --model claude-sonnet-5-5
    python3 report.py results/<run>.jsonl
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
# A suite is a directory with app/, simulate.py, scenarios.py and checks/.
# Results from every suite go to results/, so suites can be compared.
SUITES = {"v1": HERE / "v1", "v2": HERE / "v2"}


def load_scenarios(suite: Path):
    import importlib.util
    spec = importlib.util.spec_from_file_location(f"scenarios_{suite.name}", suite / "scenarios.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod
REPO = HERE.parent

PROMPT = """You're on call for the shop service; its Python code is in the current directory. {symptom}

{logs}

Find the root cause and fix it in the code with the smallest change that makes the service correct. You can't run the service or the tests. When you're done, reply with one sentence naming the cause."""

LOGS = {
    "raw": "The service's production log for the last 26 hours is at {log}.",
    "starlist": "The service's production logs for the last 26 hours are available through the starlist MCP tools.",
    "none": "The service's logs aren't available.",
}

# With --no-hint the agent isn't told the symptom and has to find the problem in the logs.
NO_HINT = "An alert fired after the latest release, but nobody knows yet what is wrong."

BASE_TOOLS = ["Read", "Edit", "Grep", "Glob"]


def prepare(suite: Path, work: Path, scenario: str, arm: str, seed: int, starlist: Path,
            sim_args: list, hint: bool, tools: str = "all") -> dict:
    scenarios = load_scenarios(suite)
    ws, logs = work / "ws", work / "logs"
    shutil.copytree(suite / "app", ws, ignore=shutil.ignore_patterns("__pycache__"))
    scenarios.apply(ws, scenario)
    if arm == "none":  # control: can the bug be found from the code alone?
        args = ["--tools", ",".join(BASE_TOOLS), "--strict-mcp-config",
                "--allowedTools", ",".join(BASE_TOOLS)]
        symptom = scenarios.SCENARIOS[scenario]["symptom"] if hint else NO_HINT
        return {"ws": ws, "args": args,
                "prompt": PROMPT.format(symptom=symptom, logs=LOGS["none"])}
    logs.mkdir()
    log = logs / "app.log"
    subprocess.run([sys.executable, str(suite / "simulate.py"), "--scenario", scenario,
                    "--out", str(log), "--seed", str(seed), *sim_args], check=True)
    args = ["--tools", ",".join(BASE_TOOLS), "--strict-mcp-config"]
    if arm == "raw":
        args += ["--allowedTools", ",".join(BASE_TOOLS), "--add-dir", str(logs)]
        logs_text = LOGS["raw"].format(log=log)
    else:
        db = work / "starlist.db"
        subprocess.run([str(starlist), "ingest", str(log), "--db", str(db), "--source-root", "/app"],
                       check=True, capture_output=True)
        shutil.rmtree(logs)  # the agent only gets what starlist serves
        config = {"mcpServers": {"starlist": {
            "command": str(starlist), "args": ["mcp", "--stdio", "--db", str(db), "--no-alerts", "--tools", tools]}}}
        (work / "mcp.json").write_text(json.dumps(config))
        args += ["--mcp-config", str(work / "mcp.json"),
                 "--allowedTools", ",".join(BASE_TOOLS + ["mcp__starlist"])]
        logs_text = LOGS["starlist"]
    symptom = scenarios.SCENARIOS[scenario]["symptom"] if hint else NO_HINT
    prompt = PROMPT.format(symptom=symptom, logs=logs_text)
    return {"ws": ws, "args": args, "prompt": prompt}


def grade(suite: Path, work: Path, scenario: str) -> dict:
    """Run the visible tests and the hidden check on a copy of what the agent left."""
    g = work / "grade"
    shutil.rmtree(g, ignore_errors=True)
    shutil.copytree(work / "ws", g, ignore=shutil.ignore_patterns("__pycache__"))
    shutil.copy(suite / "checks" / f"{scenario}.py", g / "tests" / "test_hidden_check.py")
    for helper in (suite / "checks").glob("fake_*.py"):
        shutil.copy(helper, g / "tests" / helper.name)
    env = {**os.environ, "PYTHONPATH": str(g)}
    r = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests"],
                       cwd=g, env=env, capture_output=True, text=True, timeout=120)
    return {"fixed": r.returncode == 0, "test_output": r.stderr[-1500:]}


def tool_name_of(block_id: str, calls: dict) -> str:
    return calls.get(block_id, {}).get("name", "?")


def measure(transcript: Path) -> dict:
    """Token use from the agent's stream, plus how much of the context came from logs."""
    calls, seen, peak = {}, set(), 0
    log_chars, log_calls, other_tool_chars = 0, 0, 0
    result = {}
    for raw in transcript.read_text().splitlines():
        try:
            ev = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if ev.get("type") == "assistant":
            msg = ev["message"]
            for b in msg.get("content", []):
                if b.get("type") == "tool_use":
                    calls[b["id"]] = {"name": b["name"], "input": b.get("input", {})}
            if msg.get("id") not in seen:
                seen.add(msg.get("id"))
                u = msg.get("usage", {})
                peak = max(peak, u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0)
                           + u.get("cache_creation_input_tokens", 0))
        elif ev.get("type") == "user":
            content = ev.get("message", {}).get("content", [])
            for b in content if isinstance(content, list) else []:
                if b.get("type") != "tool_result":
                    continue
                c = b.get("content")
                text = c if isinstance(c, str) else "".join(
                    x.get("text", "") for x in (c or []) if isinstance(x, dict))
                call = calls.get(b.get("tool_use_id"), {})
                name, inp = call.get("name", "?"), json.dumps(call.get("input", {}))
                if name.startswith("mcp__starlist") or "/logs/" in inp:
                    log_chars += len(text)
                    log_calls += 1
                else:
                    other_tool_chars += len(text)
        elif ev.get("type") == "result":
            result = ev
    usage = {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0}
    for m in result.get("modelUsage", {}).values():
        usage["input"] += m.get("inputTokens", 0)
        usage["output"] += m.get("outputTokens", 0)
        usage["cache_read"] += m.get("cacheReadInputTokens", 0)
        usage["cache_write"] += m.get("cacheCreationInputTokens", 0)
    return {
        "tokens": usage,
        "total_tokens": sum(usage.values()),
        "cost_usd": result.get("total_cost_usd"),
        "turns": result.get("num_turns"),
        "duration_s": round(result.get("duration_ms", 0) / 1000),
        "peak_context": peak,
        "log_tool_calls": log_calls,
        "log_output_chars": log_chars,
        "other_tool_output_chars": other_tool_chars,
        "answer": (result.get("result") or "")[:600],
        "stop": result.get("subtype"),
        "tools_used": sorted({c["name"] for c in calls.values()}),
    }


def run_one(job: dict) -> dict:
    work = Path(job["work"])
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    suite = Path(job["suite_dir"])
    p = prepare(suite, work, job["scenario"], job["arm"], job["seed"], Path(job["starlist"]),
                job["sim_args"], job["hint"], job.get("tools", "all"))
    log = work / "logs" / "app.log"
    log_bytes = log.stat().st_size if log.exists() else None
    transcript = work / "transcript.jsonl"
    cmd = ["claude", "-p", p["prompt"], "--output-format", "stream-json", "--verbose",
           "--model", job["model"], "--setting-sources", "", "--max-turns", str(job["max_turns"]),
           *p["args"]]
    started = time.time()
    with open(transcript, "w") as out:
        try:
            subprocess.run(cmd, cwd=p["ws"], stdout=out, stderr=subprocess.STDOUT,
                           timeout=job["timeout"], stdin=subprocess.DEVNULL)
            timed_out = False
        except subprocess.TimeoutExpired:
            timed_out = True
    row = {k: job[k] for k in ("suite", "variant", "scenario", "arm", "seed", "model")}
    row.update(measure(transcript))
    row.update(grade(suite, work, job["scenario"]))
    if log_bytes:
        row["log_bytes"] = log_bytes
    if job["arm"] == "starlist" and job.get("tools", "all") != "all":
        row["starlist_tools"] = job["tools"]
    if not job["keep"]:  # logs and databases are large; keep the transcript and the edit
        shutil.rmtree(work / "logs", ignore_errors=True)
        for f in work.glob("starlist.db*"):
            f.unlink()
    row["timed_out"] = timed_out
    row["wall_s"] = round(time.time() - started)
    print(f"{row['scenario']:<18} {row['arm']:<9} seed={row['seed']} fixed={row['fixed']!s:<5} "
          f"tokens={row['total_tokens']:>8} log_chars={row['log_output_chars']:>7} "
          f"turns={row['turns']}", flush=True)
    return row


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--suite", choices=sorted(SUITES), default="v1",
                   help="v1: bugs visible in the code; v2: bugs only the logs reveal")
    p.add_argument("--model", default="claude-sonnet-5-5")
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--scenario", action="append", help="Only these (repeatable)")
    p.add_argument("--arm", action="append", choices=["raw", "starlist", "none"])
    p.add_argument("--parallel", type=int, default=4)
    p.add_argument("--max-turns", type=int, default=40)
    p.add_argument("--timeout", type=int, default=1200, help="Seconds per agent run")
    p.add_argument("--work", default=os.environ.get("BENCH_WORK", "/tmp/starlist-bench"))
    p.add_argument("--starlist", default=str(REPO / "target" / "release" / "starlist"))
    p.add_argument("--large", action="store_true",
                   help="Large, noisy logs: 8x traffic with DEBUG lines (about 300 MB)")
    p.add_argument("--no-hint", action="store_true",
                   help="Don't tell the agent the symptom; it has to find it in the logs")
    p.add_argument("--keep", action="store_true", help="Keep logs and databases after each run")
    p.add_argument("--tools", default="all",
                   help="starlist's --tools: all, core or a comma-separated list")
    p.add_argument("--out", help="Results file (default results/<time>-<model>.jsonl)")
    a = p.parse_args()

    if not Path(a.starlist).exists():
        sys.exit(f"build starlist first: cargo build --release ({a.starlist} not found)")
    stamp = time.strftime("%Y%m%d-%H%M%S")
    variant = ("large" if a.large else "small") + ("-no-hint" if a.no_hint else "")
    sim_args = ["--traffic", "8", "--debug"] if a.large else []
    out = Path(a.out or HERE / "results" / f"{stamp}-{a.suite}-{variant}-{a.model}.jsonl")
    out.parent.mkdir(exist_ok=True)
    jobs = []
    for seed in range(1, a.repeats + 1):
        for scenario in a.scenario or sorted(load_scenarios(SUITES[a.suite]).SCENARIOS):
            for arm in a.arm or ["raw", "starlist"]:
                jobs.append({"variant": variant, "sim_args": sim_args, "keep": a.keep,
                             "hint": not a.no_hint, "suite": a.suite,
                             "suite_dir": str(SUITES[a.suite]),
                             "scenario": scenario, "arm": arm, "seed": seed, "model": a.model,
                             "starlist": a.starlist, "max_turns": a.max_turns,
                             "timeout": a.timeout, "tools": a.tools,
                             "work": f"{a.work}/{stamp}-{a.suite}-{variant}/{scenario}-{arm}-{seed}"})
    with ThreadPoolExecutor(a.parallel) as pool, open(out, "a") as f:
        for row in pool.map(run_one, jobs):
            f.write(json.dumps(row) + "\n")
            f.flush()
    print(f"results: {out}")


if __name__ == "__main__":
    main()
