"""Round 2 log simulator: the shop runs for 26 hours against simulated upstream services
(database, payment gateway, carrier, mail relay) and simulated clients. About 40 minutes
before the end, something outside the service changes, as each scenario describes.

    python3 v2/simulate.py --scenario carrier-format --out app.log
"""

import argparse
import logging
import random
import shutil
import sys
import tempfile
import time
from decimal import Decimal
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name: str, path: Path):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


sys.path.insert(0, str(HERE.parent / "v1"))  # v1's simulate imports its scenarios
base = _load("v1_simulate", HERE.parent / "v1" / "simulate.py")  # formatter, traffic, helpers
sys.path.remove(str(HERE.parent / "v1"))
round2 = _load("v2_scenarios", HERE / "scenarios.py")


class World:
    """Everything outside the service, with an incident starting at `incident_at`."""

    def __init__(self, scenario: str, incident_at: float, rng: random.Random, clock):
        self.scenario, self.incident_at, self.rng, self.clock = scenario, incident_at, rng, clock
        self.products, self.stock = {}, {}
        names = ["mug", "tee", "hoodie", "cap", "poster", "sticker pack", "tote", "notebook",
                 "socks", "beanie", "water bottle", "pin set"]
        for i in range(120):
            sku = 1000 + i
            self.products[sku] = {"sku": sku, "name": f"{names[i % len(names)]} #{i}",
                                  "price": Decimal(5 + (i * 37) % 80) + Decimal("0.99"),
                                  "weight_g": 80 + (i * 53) % 900}
            self.stock[sku] = 12 + (i * 7) % 30
        # product-data: the catalog import that drops weights for a few popular products.
        self.no_weight = {1004, 1017, 1031, 1052, 1088}

    def incident(self) -> bool:
        return self.clock.now() >= self.incident_at

    def _wait(self, ms: float) -> None:
        self.clock.advance(ms / 1000)

    # -- database ---------------------------------------------------------------
    def db(self, sql: str, params):
        r = self.rng
        ms = 4 + r.expovariate(1 / 30) + (r.uniform(400, 1500) if r.random() < 0.04 else 0)
        self._wait(ms)
        if sql.startswith("SELECT * FROM products"):
            p = self.products.get(params[0])
            if not p:
                return []
            row = dict(p)
            if self.scenario == "product-data" and self.incident() and row["sku"] in self.no_weight:
                row["weight_g"] = None
            return [row]
        if sql.startswith("SELECT stock"):
            return [{"stock": self.stock.get(params[0], 0)}]
        if sql.startswith("UPDATE stock"):
            self.stock[params[1]] = self.stock.get(params[1], 0) - params[0]
        return []

    def restock(self) -> int:
        n = 0
        for i, sku in enumerate(self.products):
            if self.stock[sku] < 8:
                self.stock[sku] = 15 + (i * 7 + int(self.clock.now())) % 30
                n += 1
        return n

    # -- HTTP services ------------------------------------------------------------
    def http(self, service, method, path, payload, timeout_ms):
        r = self.rng
        if service == "gateway":
            if r.random() < 0.006:
                self._wait(timeout_ms)
                return 0, {}
            self._wait(120 + r.expovariate(1 / 80))
            if self.scenario == "gateway-rate-limit" and self.incident() and r.random() < 0.18:
                return 200, {"status": "failed", "code": "rate_limited",
                                   "message": "Too many requests, retry after 500ms",
                                   "retry_after_ms": 500}
            if r.random() < 0.004:
                return 200, {"status": "failed", "code": "processing_error"}
            if r.random() < 0.015:
                return 200, {"status": "failed", "code": "insufficient_funds"}
            return 200, {"status": "succeeded",
                         "id": "ch_" + "".join(r.choice("abcdef0123456789") for _ in range(16))}
        if service == "carrier":
            ms = 40 + r.expovariate(1 / 60)
            self._wait(ms)
            if r.random() < 0.002:
                return 503, {"error": "upstream unavailable"}
            zone, kg = payload["zone"], Decimal(payload["weight_kg"])
            price = (Decimal({1: "3.50", 2: "6.00", 3: "9.50", 4: "14.00"}[zone])
                     + Decimal("1.20") * kg).quantize(Decimal("0.01"))
            if self.scenario == "carrier-format" and self.incident():
                return 200, {"rate": {"amount_cents": int(price * 100), "currency": "USD"},
                             "service": "standard", "api_version": "2026-10"}
            return 200, {"amount": str(price), "currency": "USD", "service": "standard"}
        if service == "mail":
            self._wait(30 + r.expovariate(1 / 40))
            if r.random() < 0.004:
                return 503, {"error": "Connection unexpectedly closed: smtp.internal:587"}
            return 202, {"queued": True}
        raise ValueError(service)


AGENTS = [("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", 45), ("Mozilla/5.0 (Macintosh)", 20),
          ("ShopApp/4.9.2 (iOS 18.1)", 20), ("ShopApp/4.9.2 (Android 15)", 15)]


def user_agent(world: World, rng: random.Random) -> str:
    ua = rng.choices([a for a, _ in AGENTS], [w for _, w in AGENTS])[0]
    if world.scenario == "mobile-qty" and world.incident() and ua.startswith("ShopApp/4.9.2 (iOS"):
        # The new iOS release rolls out over the incident window.
        share = min(1.0, (world.clock.now() - world.incident_at) / 1800)
        if rng.random() < 0.3 + 0.6 * share:
            ua = "ShopApp/5.0.0 (iOS 18.1)"
    return ua


def run(mods, world, log, rng, start, end, traffic):
    clock, app, catalog = mods["clock"], mods["app"], mods["catalog"]
    clock.set_now(start)
    log.info("Starting shop version 2.4.0 on :8080 (workers=4, pid=%d)", rng.randrange(20, 60000))
    log.info("connected to db primary=db-1.internal pool=10")
    skus = list(world.products)
    weights = [1 / (i + 1) ** 0.8 for i in range(len(skus))]
    rng.shuffle(weights)
    # The products that lose their weight are popular ones.
    for sku in world.no_weight:
        weights[skus.index(sku)] = max(weights) * 0.6
    t = start + 1
    nh, nm, nr, nmet = t, t + 60, t + 1800, t + 300
    jobs = logging.getLogger("shop.jobs")
    while True:
        t += rng.expovariate(base.traffic_rate(t) * traffic)
        if t >= end:
            break
        while nh <= t:
            clock.set_now(nh)
            app.handle("GET", "/healthz", user_agent="kube-probe/1.31")
            nh += 15
        while nm <= t:
            clock.set_now(nm)
            catalog.report_cache_stats()
            nm += 60
        while nr <= t:
            clock.set_now(nr)
            jobs.info("restock job: replenished %d products", world.restock())
            nr += 1800
        while nmet <= t:
            clock.set_now(nmet)
            jobs.info("metrics flushed to statsd series=%d", rng.randrange(180, 220))
            nmet += 300
        clock.set_now(t)
        ua = user_agent(world, rng)
        if rng.random() < 0.72:
            sku = rng.choices(skus, weights)[0] if rng.random() > 0.01 else 9999
            app.handle("GET", f"/products/{sku}", user_agent=ua)
        else:
            body = base.checkout_body(rng, skus)
            body.pop("gift_card", None)
            if ua.startswith("ShopApp/5."):
                for item in body["items"]:
                    item["qty"] = str(item["qty"])
            app.handle("POST", "/checkout", body, user_agent=ua)


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--scenario", required=True, choices=sorted(round2.SCENARIOS))
    p.add_argument("--out", required=True)
    p.add_argument("--hours", type=float, default=26)
    p.add_argument("--end", type=float)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--traffic", type=float, default=1.0)
    p.add_argument("--debug", action="store_true")
    a = p.parse_args()
    end = a.end or time.time()
    start = end - a.hours * 3600
    with tempfile.TemporaryDirectory() as tmp:
        app_dir = Path(tmp) / "app"
        shutil.copytree(HERE / "app" / "shop", app_dir / "shop")
        handler = logging.FileHandler(a.out, mode="w")
        root = logging.getLogger()
        root.setLevel(logging.DEBUG if a.debug else logging.INFO)
        root.addHandler(handler)
        handler.addFilter(base.ClockStamp())
        handler.setFormatter(base.Formatter(app_dir))
        mods = base.load_version(app_dir)
        mods["rand"].seed(a.seed)
        rng = random.Random(a.seed)
        world = World(a.scenario, end - round2.INCIDENT_MINUTES_AGO * 60, random.Random(a.seed + 7),
                      mods["clock"])
        mods["db"].connection = world.db
        sys.modules["shop.clients"].transport = world.http
        run(mods, world, logging.getLogger("shop.main"), rng, start, end, a.traffic)
        handler.close()


if __name__ == "__main__":
    main()
