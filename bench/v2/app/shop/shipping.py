from decimal import ROUND_HALF_UP, Decimal

from . import clients
from .config import SETTINGS, ZONES
from .log import get_logger

log = get_logger("shop.shipping")


def total_weight_kg(lines) -> Decimal:
    grams = sum(line["weight_g"] * line["qty"] for line in lines)
    return Decimal(grams) / 1000


def _carrier_quote(zone: int, weight_kg: Decimal) -> Decimal:
    body = clients.call("carrier", "POST", "/v2/rates",
                        {"zone": zone, "weight_kg": str(weight_kg), "service": "standard"})
    try:
        return Decimal(body["amount"]).quantize(Decimal("0.01"), ROUND_HALF_UP)
    except Exception:
        log.exception("unexpected carrier response: %s", body)
        raise


def quote(lines, country: str, items_total: Decimal) -> Decimal:
    if items_total >= SETTINGS["FREE_SHIPPING_OVER"]:
        return Decimal("0")
    zone = ZONES.get(country)
    if zone is None:
        return SETTINGS["FLAT_SHIPPING"]
    weight = total_weight_kg(lines)
    try:
        return _carrier_quote(zone, weight)
    except Exception as e:
        log.warning("carrier quote failed (%s), using flat rate %s", type(e).__name__,
                    SETTINGS["FLAT_SHIPPING"])
        return SETTINGS["FLAT_SHIPPING"]
