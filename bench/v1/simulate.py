"""Run the shop service against simulated traffic and write its log.

The old version runs for most of the period, then the release (with the scenario's bug)
is deployed and runs until the end. Log lines, stack traces and timings all come from
actually executing the service code, with a simulated clock.

    python3 simulate.py --scenario coupon-percent --out app.log
"""

import argparse
import importlib
import logging
import random
import shutil
import sys
import tempfile
import time
import traceback
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path

import scenarios

HERE = Path(__file__).resolve().parent


class Formatter(logging.Formatter):
    """`2026-11-06T17:01:02.123Z INFO shop.orders request_id=... message`, with stack
    traces shown as if the service ran from /app (as it would in a container)."""

    def __init__(self, deployed: Path):
        super().__init__()
        self.deployed = str(deployed)

    def format(self, record):
        ts = datetime.fromtimestamp(record.created, timezone.utc)
        stamp = ts.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ts.microsecond // 1000:03d}Z"
        rid = getattr(record, "request_id", "-")
        rid = f" request_id={rid}" if rid != "-" else ""
        line = f"{stamp} {record.levelname} {record.name}{rid} {record.getMessage()}"
        if record.exc_info:
            tb = "".join(traceback.format_exception(*record.exc_info)).rstrip()
            line += "\n" + tb.replace(self.deployed, "/app")
        return line


class ClockStamp(logging.Filter):
    def filter(self, record):
        record.created = sys.modules["shop.clock"].now()
        return True


def load_version(app_dir: Path):
    for name in [m for m in sys.modules if m == "shop" or m.startswith("shop.")]:
        del sys.modules[name]
    sys.path.insert(0, str(app_dir))
    try:
        mods = {n: importlib.import_module(f"shop.{n}") for n in
                ["clock", "rand", "app", "catalog", "db", "config"]}
    finally:
        sys.path.remove(str(app_dir))
    return mods


def traffic_rate(ts: float) -> float:
    """Requests per second, following the time of day (UTC)."""
    hour = datetime.fromtimestamp(ts, timezone.utc).hour + datetime.fromtimestamp(ts, timezone.utc).minute / 60
    import math
    return 0.25 + 0.55 * (1 + math.cos((hour - 15) / 24 * 2 * math.pi)) / 2 * 1.6


COUNTRIES = [("US", "USD", 40), ("CA", "CAD", 10), ("GB", "GBP", 14), ("DE", "EUR", 14),
             ("FR", "EUR", 9), ("NL", "EUR", 5), ("AU", "AUD", 5), ("NZ", "USD", 3)]
COUPONS = ["SAVE10", "SAVE15", "WELCOME5", "VIP20"]


def checkout_body(rng: random.Random, skus: list[int]) -> dict:
    n = rng.choices([1, 2, 3], [60, 30, 10])[0]
    items = []
    for sku in rng.sample(skus, n):
        items.append({"sku": sku, "qty": rng.choices([1, 2, 3], [70, 22, 8])[0]})
    if rng.random() < 0.005:
        # A product that was deleted while it sat in someone's cart.
        items[0]["sku"] = 9999
    country, currency, _ = rng.choices(COUNTRIES, [c[2] for c in COUNTRIES])[0]
    body = {
        "items": items,
        "email": f"user{rng.randrange(4000)}@example.com",
        "country": country,
        "currency": currency,
        "card_token": "tok_" + "".join(rng.choice("abcdefghijklmnop0123456789") for _ in range(20)),
    }
    if rng.random() < 0.09:
        body["coupon"] = rng.choice(COUPONS)
    if rng.random() < 0.05:
        body["gift_card"] = {"code": f"GC-{rng.randrange(1000, 9999)}-{rng.randrange(1000, 9999)}",
                             "amount": Decimal(rng.choice([10, 25, 50])) + Decimal("0.00")}
    return body


def run(mods, log: logging.Logger, rng: random.Random, start: float, end: float, version: str,
        first: bool, traffic: float):
    clock, app, catalog, db = mods["clock"], mods["app"], mods["catalog"], mods["db"]
    clock.set_now(start)
    log.info("Starting shop version %s on :8080 (workers=4, pid=%d)", version, rng.randrange(20, 60000))
    log.info("connected to db primary=db-1.internal pool=10")
    skus = list(db.PRODUCTS)
    weights = [1 / (i + 1) ** 0.8 for i in range(len(skus))]
    rng.shuffle(weights)
    t = start + 1
    next_health, next_minute, next_restock, next_metrics = t, t + 60, t + 1800, t + 300
    while True:
        t += rng.expovariate(traffic_rate(t) * traffic)
        if t >= end:
            break
        while next_health <= t:
            clock.set_now(next_health)
            app.handle("GET", "/healthz")
            next_health += 15
        while next_minute <= t:
            clock.set_now(next_minute)
            catalog.report_cache_stats()
            next_minute += 60
        while next_restock <= t:
            clock.set_now(next_restock)
            log.info("restock job: replenished %d products", db.restock())
            next_restock += 1800
        while next_metrics <= t:
            clock.set_now(next_metrics)
            log.info("metrics flushed to statsd series=%d", rng.randrange(180, 220))
            next_metrics += 300
        clock.set_now(t)
        if rng.random() < 0.72:
            sku = rng.choices(skus, weights)[0] if rng.random() > 0.01 else 9999
            app.handle("GET", f"/products/{sku}")
        else:
            app.handle("POST", "/checkout", checkout_body(rng, skus))
    if not first:
        return
    clock.set_now(end)
    log.info("received SIGTERM, draining connections")
    log.info("shutdown complete")


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--scenario", required=True, choices=sorted(scenarios.SCENARIOS))
    p.add_argument("--out", required=True)
    p.add_argument("--hours", type=float, default=26)
    p.add_argument("--deploy-minutes-ago", type=float, default=45)
    p.add_argument("--end", type=float, help="Unix time of the last line (default: now)")
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--traffic", type=float, default=1.0, help="Traffic multiplier")
    p.add_argument("--debug", action="store_true", help="Log at DEBUG, as some services do")
    p.add_argument("--keep-release", help="Also copy the released code here")
    a = p.parse_args()

    end = a.end or time.time()
    deploy = end - a.deploy_minutes_ago * 60
    start = end - a.hours * 3600

    with tempfile.TemporaryDirectory() as tmp:
        old = Path(tmp) / "old" / "app"
        new = Path(tmp) / "new" / "app"
        shutil.copytree(HERE / "app" / "shop", old / "shop")
        shutil.copytree(HERE / "app" / "shop", new / "shop")
        scenarios.apply(new, a.scenario)
        if a.keep_release:
            shutil.copytree(new, a.keep_release, dirs_exist_ok=True)

        handler = logging.FileHandler(a.out, mode="w")
        root = logging.getLogger()
        root.setLevel(logging.DEBUG if a.debug else logging.INFO)
        root.addHandler(handler)
        handler.addFilter(ClockStamp())
        log = logging.getLogger("shop.main")

        rng = random.Random(a.seed)
        handler.setFormatter(Formatter(old))
        mods = load_version(old)
        mods["rand"].seed(a.seed)
        run(mods, log, rng, start, deploy - 3, scenarios.BASE_VERSION, True, a.traffic)

        stock = dict(mods["db"].STOCK)  # the database outlives the deploy
        handler.setFormatter(Formatter(new))
        mods = load_version(new)
        mods["db"].STOCK.update(stock)
        mods["rand"].seed(a.seed + 1)
        run(mods, log, rng, deploy, end, scenarios.RELEASE_VERSION, False, a.traffic)
        handler.close()


if __name__ == "__main__":
    main()
