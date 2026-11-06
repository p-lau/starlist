from . import clock, db
from .config import SETTINGS
from .log import get_logger

log = get_logger("shop.catalog")

_cache = {}
_stats = {"hits": 0, "misses": 0}


class NotFound(Exception):
    pass


def get_product(sku: int) -> dict:
    ttl = SETTINGS.get("CACHE_TTL_SECONDS", 0)
    hit = _cache.get(sku)
    if hit and clock.now() - hit[0] < ttl:
        _stats["hits"] += 1
        log.debug("cache hit sku=%d", sku)
        return hit[1]
    _stats["misses"] += 1
    log.debug("cache miss sku=%d", sku)
    rows = db.query("SELECT * FROM products WHERE sku = ?", sku)
    if not rows:
        raise NotFound(sku)
    _cache[sku] = (clock.now(), rows[0])
    return rows[0]


def report_cache_stats() -> None:
    """Called once a minute by the scheduler."""
    total = _stats["hits"] + _stats["misses"]
    rate = 100 * _stats["hits"] // total if total else 100
    log.info("product cache: entries=%d lookups=%d hit_rate=%d%%", len(_cache), total, rate)
    _stats["hits"] = _stats["misses"] = 0


def clear_cache() -> None:
    _cache.clear()
