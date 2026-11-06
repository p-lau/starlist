from decimal import Decimal

from . import clients
from .config import SETTINGS
from .log import get_logger

log = get_logger("shop.payments")

# Gateway failure codes worth another attempt; everything else is a decline.
RETRYABLE = {"processing_error"}


class GatewayTimeout(Exception):
    pass


class PaymentDeclined(Exception):
    pass


def _gateway_charge(amount: Decimal, currency: str, card_token: str) -> str:
    log.debug("gateway request amount=%s currency=%s card=%s", amount, currency, card_token)
    try:
        body = clients.call("gateway", "POST", "/v1/charges",
                            {"amount": str(amount), "currency": currency, "source": card_token},
                            timeout_ms=SETTINGS["GATEWAY_TIMEOUT_MS"])
    except clients.Timeout as e:
        raise GatewayTimeout(str(e))
    if body.get("status") == "succeeded":
        return body["id"]
    code = body.get("code", "unknown")
    if code in RETRYABLE:
        raise GatewayTimeout(f"gateway error {code}")
    raise PaymentDeclined(f"card declined: {code}")


def charge(amount: Decimal, currency: str, card_token: str) -> str:
    if amount <= 0:
        raise ValueError(f"charge amount must be positive, got {amount}")
    attempts = 1 + SETTINGS["PAYMENT_RETRIES"]
    for attempt in range(1, attempts + 1):
        try:
            charge_id = _gateway_charge(amount, currency, card_token)
            log.info("charged %s %s charge=%s", amount, currency, charge_id)
            return charge_id
        except GatewayTimeout as e:
            if attempt == attempts:
                raise
            log.warning("gateway: %s, retrying (attempt %d of %d)", e, attempt, attempts)
    raise AssertionError("unreachable")
