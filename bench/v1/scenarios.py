"""The seeded bugs. Each scenario is the release that went out (shared edits plus one bug),
the symptom the on-call agent is told about, and a hidden check that passes once it's fixed.

Edits are (file, old, new) replacements applied to app/ to build the released version.
"""

BASE_VERSION = "2.3.4"
RELEASE_VERSION = "2.4.0"

# Changes every release carries, so the bug isn't the only difference: version bump and
# Australia as a new market.
RELEASE = [
    ("shop/__init__.py", 'VERSION = "2.3.4"', 'VERSION = "2.4.0"'),
    ("shop/config.py", '    "CAD": Decimal("1.36"),\n', '    "CAD": Decimal("1.36"),\n    "AUD": Decimal("1.53"),\n'),
    ("shop/config.py", '"NL": 3}', '"NL": 3, "AU": 4}'),
    ("shop/config.py", '    "NL": Decimal("0.21"),\n', '    "NL": Decimal("0.21"),\n    "AU": Decimal("0.10"),\n'),
    ("shop/shipping.py", '3: Decimal("9.50")}', '3: Decimal("9.50"), 4: Decimal("14.00")}'),
]

SCENARIOS = {
    "coupon-percent": {
        "summary": "Coupons were changed to whole percents in config; pricing still treats them as fractions, so totals go negative and the charge fails.",
        "symptom": "Some checkouts have been failing with a server error since the latest release. Most go through.",
        "edits": [
            ("shop/config.py", "# Coupon code -> fraction taken off the subtotal.", "# Coupon code -> percent off the subtotal (same as the admin UI)."),
            ("shop/config.py", '"SAVE10": Decimal("0.10")', '"SAVE10": Decimal("10")'),
            ("shop/config.py", '"SAVE15": Decimal("0.15")', '"SAVE15": Decimal("15")'),
            ("shop/config.py", '"WELCOME5": Decimal("0.05")', '"WELCOME5": Decimal("5")'),
            ("shop/config.py", '"VIP20": Decimal("0.20")', '"VIP20": Decimal("20")'),
        ],
    },
    "shipping-weight": {
        "summary": "A new grams-to-kg helper divides by a million, so every carrier quote is rejected as underweight and orders fall back to the flat rate.",
        "symptom": "Support says shipping prices on orders look wrong since the latest release. Nothing is crashing.",
        "edits": [
            ("shop/shipping.py", 'MIN_WEIGHT_KG = Decimal("0.05")\n', 'MIN_WEIGHT_KG = Decimal("0.05")\nPACKAGING_G = 150\n'),
            ("shop/shipping.py",
             '    grams = sum(line["weight_g"] * line["qty"] for line in lines)\n    return Decimal(grams) / 1000\n',
             '    grams = sum(line["weight_g"] * line["qty"] for line in lines) + PACKAGING_G\n    return _kg(grams)\n\n\ndef _kg(grams: int) -> Decimal:\n    """Grams to kilograms, to the gram."""\n    return (Decimal(grams) / 1_000_000).quantize(Decimal("0.001"))\n'),
        ],
    },
    "receipt-giftcard": {
        "summary": "Gift card support added a receipt line that adds a Decimal to a str; the TypeError is logged under the same message as old, unrelated SMTP failures.",
        "symptom": "A few customers say they never got their receipt email for orders placed since the latest release.",
        "edits": [
            ("shop/orders.py", '"charge": charge_id}', '"charge": charge_id, "gift_card": req.get("gift_card")}'),
            ("shop/receipts.py",
             '    lines.append(f"Shipping: {order[\'shipping\']}")\n',
             '    lines.append(f"Shipping: {order[\'shipping\']}")\n    card = order.get("gift_card")\n    if card:\n        lines.append("Gift card ****" + card["code"][-4:] + " applied: -" + card["amount"])\n'),
        ],
    },
    "cache-config": {
        "summary": "The cache TTL setting was renamed in config but catalog still reads the old name with a default of 0, so the product cache never hits.",
        "symptom": "Product pages have been slow since the latest release.",
        "edits": [
            ("shop/config.py", '    "CACHE_TTL_SECONDS": 300,\n', '    "CATALOG_CACHE_TTL_SECONDS": 300,\n    "SEARCH_CACHE_TTL_SECONDS": 60,\n'),
        ],
    },
    "stock-boundary": {
        "summary": "The stock check became stock > qty, so buying the last units of a product is refused as out of stock.",
        "symptom": "Customers say checkout tells them an item is out of stock even though the product page showed it as available.",
        "edits": [
            ("shop/inventory.py", "    return stock >= qty\n", "    return stock > qty\n"),
        ],
    },
}


def apply(app_dir, scenario: str) -> None:
    """Turn a copy of app/ into the released (buggy) version for a scenario."""
    from pathlib import Path

    for rel, old, new in RELEASE + SCENARIOS[scenario]["edits"]:
        path = Path(app_dir) / rel
        text = path.read_text()
        if text.count(old) != 1:
            raise ValueError(f"{scenario}: edit for {rel} doesn't match exactly once: {old!r}")
        path.write_text(text.replace(old, new))
