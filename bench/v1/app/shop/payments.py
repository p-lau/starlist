from decimal import Decimal

from . import clock, rand
from .config import SETTINGS
from .log import get_logger

log = get_logger("shop.payments")


class GatewayTimeout(Exception):
    pass


class PaymentDeclined(Exception):
    pass


def _gateway_charge(amount: Decimal, currency: str, card_token: str) -> str:
    """The payment provider's API (simulated)."""
    log.debug("gateway request amount=%s currency=%s card=%s", amount, currency, card_token)
    r = rand.rng.random()
    if r < 0.006:
        clock.advance(SETTINGS["GATEWAY_TIMEOUT_MS"] / 1000)
        raise GatewayTimeout(f"no response from gateway after {SETTINGS['GATEWAY_TIMEOUT_MS']}ms")
    clock.advance((120 + rand.rng.expovariate(1 / 80)) / 1000)
    if r < 0.02:
        raise PaymentDeclined("card declined: insufficient_funds")
    return "ch_" + "".join(rand.rng.choice("abcdef0123456789") for _ in range(16))


def charge(amount: Decimal, currency: str, card_token: str) -> str:
    if amount <= 0:
        raise ValueError(f"charge amount must be positive, got {amount}")
    attempts = 1 + SETTINGS["PAYMENT_RETRIES"]
    for attempt in range(1, attempts + 1):
        try:
            charge_id = _gateway_charge(amount, currency, card_token)
            log.info("charged %s %s charge=%s", amount, currency, charge_id)
            return charge_id
        except GatewayTimeout:
            if attempt == attempts:
                raise
            log.warning("gateway timeout, retrying (attempt %d of %d)", attempt, attempts)
    raise AssertionError("unreachable")
