"""Load test: sustained high log volume with bounded memory and disk.

Writes a busy, varied log (requests, stack traces, high-cardinality values,
a share of never-seen-before lines, and file rotation) into a file that
`starlist serve` follows, samples starlist's memory and database size, and
checks that both level off.

    cargo build --release
    python bench/load.py                       # 5,000 lines/s for 3 minutes
    python bench/load.py --rate 20000 --seconds 600

Exits non-zero if a limit is exceeded or ingestion falls behind. Standard
library only; runs on Linux, macOS and Windows.
"""

import argparse
import json
import os
import random
import shutil
import string
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "starlist" + (".exe" if os.name == "nt" else ""))


def rss_mb(pid):
    """Resident memory of a process, in MB."""
    if sys.platform.startswith("linux"):
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        class Counters(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        k32 = ctypes.WinDLL("kernel32")
        psapi = ctypes.WinDLL("psapi")
        k32.OpenProcess.restype = wintypes.HANDLE
        h = k32.OpenProcess(0x1000 | 0x0010, False, pid)  # QUERY_LIMITED_INFORMATION | VM_READ
        c = Counters()
        c.cb = ctypes.sizeof(c)
        psapi.GetProcessMemoryInfo(h, ctypes.byref(c), c.cb)
        k32.CloseHandle(h)
        return c.WorkingSetSize / 2**20
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True)
    return int(out.stdout.strip() or 0) / 1024


def db_mb(db):
    return sum(os.path.getsize(db + s) for s in ("", "-wal", "-shm") if os.path.exists(db + s)) / 2**20


def status(port):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/v1/status", timeout=5) as r:
        return json.load(r)


WORDS = ["".join(random.choices(string.ascii_lowercase, k=random.randint(4, 9))) for _ in range(5000)]
CITIES = ["lisbon", "osaka", "denver", "nairobi", "oslo", "quito", "perth", "dakar"]


def line(i, unique):
    now = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    rid = uuid.uuid4().hex[:16]
    r = random.random()
    if r < unique:
        # Never-seen-before text: each line can be a new pattern.
        return f"{now} INFO " + " ".join(random.choices(WORDS, k=6)) + "\n"
    r = (r - unique) / (1 - unique)
    if r < 0.70:
        return f"{now} INFO GET /items/{random.randint(1, 10**6)} 200 request_id={rid} duration_ms={random.randint(1, 900)}\n"
    if r < 0.82:
        # One word that takes many values: these merge into one pattern.
        return f"{now} INFO user {random.choice(WORDS)} signed in from {random.choice(CITIES)} request_id={rid}\n"
    if r < 0.90:
        return f"{now} WARN cache miss for {random.choice(WORDS)} after {random.randint(1, 50)} ms\n"
    n = random.randint(1, 40)
    return (
        f"{now} ERROR charge failed for order {i} request_id={rid} card=4111111111111111\n"
        "Traceback (most recent call last):\n"
        f'  File "/app/shop/payments.py", line {n}, in charge\n'
        '  File "/usr/lib/python3/site-packages/requests/api.py", line 59, in request\n'
        f"ValueError: amount {i} out of range\n"
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--rate", type=int, default=5000, help="lines per second")
    ap.add_argument("--seconds", type=int, default=180)
    ap.add_argument("--unique", type=float, default=0.001, help="share of never-seen-before lines")
    ap.add_argument("--raw-size", default="16MB", help="starlist --raw-size")
    ap.add_argument("--rotate-mb", type=int, default=64, help="rotate the log at this size")
    ap.add_argument("--max-rss-mb", type=float, default=128)
    ap.add_argument("--max-db-mb", type=float, default=96)
    ap.add_argument("--port", type=int, default=7399)
    ap.add_argument("--exe", default=EXE)
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="starlist-load-")
    log, db = os.path.join(work, "app.log"), os.path.join(work, "load.db")
    open(log, "w").close()
    proc = subprocess.Popen(
        [args.exe, "serve", "--no-config", "--file", log, "--db", db,
         "--http", f"127.0.0.1:{args.port}", "--raw-size", args.raw_size],
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
    )
    for _ in range(50):
        try:
            status(args.port)
            break
        except OSError:
            time.sleep(0.2)
    else:
        proc.kill()
        sys.exit("starlist didn't start: " + proc.stderr.read())

    samples = []  # (seconds, rss, db)
    written = rotations = 0
    start = next_sample = time.monotonic()
    f = open(log, "a", buffering=1 << 20)
    try:
        while (now := time.monotonic()) - start < args.seconds:
            # Write this 100 ms slice's share of lines.
            due = int((now - start) * args.rate) - written
            if due > 0:
                f.write("".join(line(written + k, args.unique) for k in range(due)))
                f.flush()
                written += due
            if f.tell() > args.rotate_mb * 2**20:
                f.close()
                os.replace(log, log + ".1")
                f = open(log, "a", buffering=1 << 20)
                rotations += 1
            if now >= next_sample:
                samples.append((now - start, rss_mb(proc.pid), db_mb(db)))
                next_sample += 2
            time.sleep(0.1)
        f.close()
        written_at = time.monotonic()

        # Let it catch up: wait until the event count stops growing.
        last, events = -1, 0
        while time.monotonic() - written_at < 300:
            st = status(args.port)
            events = st["last_hour"]["events"]
            if events == last:
                break
            last = events
            time.sleep(2)
        lag = time.monotonic() - written_at
        st = status(args.port)
        samples.append((time.monotonic() - start, rss_mb(proc.pid), db_mb(db)))
    finally:
        proc.terminate()
        proc.wait(10)

    # Entries, not lines: a stack trace is one event.
    entries = written  # every generated entry starts with a timestamp
    third = len(samples) // 3
    mid = max(s[1] for s in samples[third : 2 * third]) if third else samples[-1][1]
    end = max(s[1] for s in samples[2 * third :])
    peak_rss = max(s[1] for s in samples)
    peak_db = max(s[2] for s in samples)
    result = {
        "rate": args.rate,
        "seconds": args.seconds,
        "entries_written": entries,
        "events_ingested_last_hour": events,
        "catch_up_seconds": round(lag, 1),
        "unique_share": args.unique,
        "rotations": rotations,
        "patterns": st["patterns"],
        "raw_events_kept": st["raw"]["events"],
        "peak_rss_mb": round(peak_rss, 1),
        "rss_mb_middle_third": round(mid, 1),
        "rss_mb_last_third": round(end, 1),
        "peak_db_mb": round(peak_db, 1),
        "final_db_mb": round(samples[-1][2], 1),
    }
    print(json.dumps(result, indent=2))
    shutil.rmtree(work, ignore_errors=True)

    failures = []
    if peak_rss > args.max_rss_mb:
        failures.append(f"memory peaked at {peak_rss:.0f} MB (limit {args.max_rss_mb:.0f})")
    if end > mid * 1.25 + 8:
        failures.append(f"memory still growing: {mid:.0f} MB in the middle third, {end:.0f} MB at the end")
    if peak_db > args.max_db_mb:
        failures.append(f"database peaked at {peak_db:.0f} MB (limit {args.max_db_mb:.0f})")
    if events < entries * 0.99:
        failures.append(f"ingested {events} of {entries} entries")
    for msg in failures:
        print("FAIL:", msg, file=sys.stderr)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
