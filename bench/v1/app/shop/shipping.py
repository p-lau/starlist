from decimal import ROUND_HALF_UP, Decimal

from . import clock, rand
from .config import SETTINGS, ZONES
from .log import get_logger

log = get_logger("shop.shipping")

BASE = {1: Decimal("3.50"), 2: Decimal("6.00"), 3: Decimal("9.50")}
PER_KG = Decimal("1.20")
MIN_WEIGHT_KG = Decimal("0.05")


class CarrierRejected(Exception):
    pass


def _carrier_quote(zone: int, weight_kg: Decimal) -> Decimal:
    """The carrier's rate API (simulated)."""
    log.debug("carrier quote request zone=%d weight_kg=%s", zone, weight_kg)
    ms = 40 + rand.rng.expovariate(1 / 60)
    clock.advance(ms / 1000)
    if ms > 800:
        log.warning("carrier api slow: %dms", ms)
    if weight_kg < MIN_WEIGHT_KG:
        raise CarrierRejected(f"weight_kg={weight_kg} below carrier minimum {MIN_WEIGHT_KG}")
    return (BASE[zone] + PER_KG * weight_kg).quantize(Decimal("0.01"), ROUND_HALF_UP)


def total_weight_kg(lines) -> Decimal:
    grams = sum(line["weight_g"] * line["qty"] for line in lines)
    return Decimal(grams) / 1000


def quote(lines, country: str, items_total: Decimal) -> Decimal:
    if items_total >= SETTINGS["FREE_SHIPPING_OVER"]:
        return Decimal("0")
    zone = ZONES.get(country)
    if zone is None:
        return SETTINGS["FLAT_SHIPPING"]
    try:
        return _carrier_quote(zone, total_weight_kg(lines))
    except CarrierRejected as e:
        log.warning("carrier rejected quote (%s), using flat rate %s", e, SETTINGS["FLAT_SHIPPING"])
        return SETTINGS["FLAT_SHIPPING"]
