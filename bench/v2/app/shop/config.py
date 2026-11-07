"""Service settings. In production these come from the environment; these are the defaults."""

from decimal import Decimal

SETTINGS = {
    "CACHE_TTL_SECONDS": 300,
    "SLOW_QUERY_MS": 500,
    "GATEWAY_TIMEOUT_MS": 30000,
    "PAYMENT_RETRIES": 1,
    "FLAT_SHIPPING": Decimal("4.99"),
    "FREE_SHIPPING_OVER": Decimal("150.00"),
    "SMTP_HOST": "smtp.internal",
}

# Coupon code -> fraction taken off the subtotal.
COUPONS = {
    "SAVE10": Decimal("0.10"),
    "SAVE15": Decimal("0.15"),
    "WELCOME5": Decimal("0.05"),
    "VIP20": Decimal("0.20"),
}

# 1 USD in each currency.
RATES = {
    "USD": Decimal("1"),
    "EUR": Decimal("0.92"),
    "GBP": Decimal("0.79"),
    "CAD": Decimal("1.36"),
}

# Destination country -> carrier zone.
ZONES = {"US": 1, "CA": 2, "GB": 3, "DE": 3, "FR": 3, "NL": 3}

TAX_RATES = {
    "US": Decimal("0.07"),
    "CA": Decimal("0.13"),
    "GB": Decimal("0.20"),
    "DE": Decimal("0.19"),
    "FR": Decimal("0.20"),
    "NL": Decimal("0.21"),
}
