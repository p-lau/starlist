"""Round 2: bugs that are not visible in the code. Nothing is deployed; something outside
the service changes about 40 minutes before the end of the log, and code that looks fine
starts failing. The fix is still a code change, graded by a hidden check."""

INCIDENT_MINUTES_AGO = 40
NO_DEPLOY = "Nothing was deployed today."

SCENARIOS = {
    "gateway-rate-limit": {
        "summary": "The payment gateway started answering some charges with code rate_limited; the service treats unknown codes as declines instead of retrying.",
        "symptom": f"Since about 40 minutes ago, more customers are told their card was declined. {NO_DEPLOY}",
    },
    "carrier-format": {
        "summary": "The carrier's rate API moved the price to rate.amount_cents; the parser expects amount, so every quote falls back to the flat rate.",
        "symptom": f"Support says shipping prices look wrong on orders from the last 40 minutes or so. {NO_DEPLOY}",
    },
    "product-data": {
        "summary": "A catalog import left some products without a weight; shipping multiplies None and checkout crashes for carts containing them.",
        "symptom": f"Some checkouts started failing with a server error about 40 minutes ago. {NO_DEPLOY}",
    },
    "mobile-qty": {
        "summary": "A new release of the mobile app sends item quantities as strings; pricing multiplies a Decimal by a str and checkout crashes for those users.",
        "symptom": f"Some checkouts started failing with a server error about 40 minutes ago. {NO_DEPLOY}",
    },
}


def apply(app_dir, scenario: str) -> None:
    """Round 2 has no code change: the released code is the code."""
    if scenario not in SCENARIOS:
        raise ValueError(scenario)
