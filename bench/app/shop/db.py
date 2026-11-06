"""Stand-in for the product database: fixed products, stock levels and query latency."""

from decimal import Decimal

from . import clock, rand
from .config import SETTINGS
from .log import get_logger

log = get_logger("shop.db")

PRODUCTS = {}
STOCK = {}
query_count = 0


def _seed_products() -> None:
    names = ["mug", "tee", "hoodie", "cap", "poster", "sticker pack", "tote", "notebook",
             "socks", "beanie", "water bottle", "pin set"]
    for i in range(120):
        sku = 1000 + i
        name = names[i % len(names)]
        price = Decimal(5 + (i * 37) % 80) + Decimal("0.99")
        PRODUCTS[sku] = {"sku": sku, "name": f"{name} #{i}", "price": price,
                         "weight_g": 80 + (i * 53) % 900}
        STOCK[sku] = 12 + (i * 7) % 30


_seed_products()


def query(sql: str, *params):
    """Run a query. Returns rows; latency is simulated and slow ones are logged."""
    global query_count
    query_count += 1
    r = rand.rng.random()
    ms = 4 + rand.rng.expovariate(1 / 30)
    if r < 0.04:
        ms += rand.rng.uniform(400, 1500)
    clock.advance(ms / 1000)
    log.debug("query %.1fms: %s params=%s", ms, sql, params)
    if ms >= SETTINGS["SLOW_QUERY_MS"]:
        log.warning("slow query %dms: %s", ms, sql)
    if sql.startswith("SELECT * FROM products"):
        product = PRODUCTS.get(params[0])
        return [dict(product)] if product else []
    if sql.startswith("SELECT stock"):
        return [{"stock": STOCK.get(params[0], 0)}]
    if sql.startswith("UPDATE stock"):
        STOCK[params[1]] = STOCK.get(params[1], 0) - params[0]
        return []
    return []


def restock() -> int:
    """Nightly-ish restock job: tops products back up."""
    n = 0
    for i, sku in enumerate(PRODUCTS):
        if STOCK[sku] < 8:
            STOCK[sku] = 15 + (i * 7 + int(clock.now())) % 30
            n += 1
    return n
