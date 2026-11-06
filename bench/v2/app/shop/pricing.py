from decimal import ROUND_HALF_UP, Decimal

from .config import COUPONS, RATES, TAX_RATES
from .log import get_logger

log = get_logger("shop.pricing")

CENT = Decimal("0.01")


class InvalidCoupon(Exception):
    pass


def subtotal(lines) -> Decimal:
    return sum((line["price"] * line["qty"] for line in lines), Decimal("0"))


def apply_coupon(amount: Decimal, code: str | None) -> Decimal:
    if not code:
        return amount
    if code not in COUPONS:
        raise InvalidCoupon(code)
    off = COUPONS[code]
    return (amount * (1 - off)).quantize(CENT, ROUND_HALF_UP)


def add_tax(amount: Decimal, country: str) -> Decimal:
    rate = TAX_RATES.get(country, Decimal("0"))
    return (amount * (1 + rate)).quantize(CENT, ROUND_HALF_UP)


def convert(amount_usd: Decimal, currency: str) -> Decimal:
    rate = RATES.get(currency)
    if rate is None:
        log.warning("unknown currency %s, charging in USD", currency)
        rate = Decimal("1")
    return (amount_usd * rate).quantize(CENT, ROUND_HALF_UP)


def order_total(lines, coupon: str | None, shipping: Decimal, country: str) -> Decimal:
    discounted = apply_coupon(subtotal(lines), coupon)
    return add_tax(discounted + shipping, country)
