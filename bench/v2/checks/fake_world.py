"""A tiny stand-in for the shop's database and upstream services, for the hidden checks."""

from decimal import Decimal

from shop import clients, clock, db

PRODUCT = {"sku": 1004, "name": "cap #4", "price": Decimal("10.00"), "weight_g": 500}


def install(product=None, gateway=None, carrier=None):
    """gateway/carrier: lists of response bodies to return in order (the last one repeats)."""
    clock.set_now(1_800_000_000)
    product = dict(PRODUCT if product is None else product)
    stock = {product["sku"]: 50}
    gateway = list(gateway or [{"status": "succeeded", "id": "ch_1"}])
    carrier = list(carrier or [{"amount": "10.70", "currency": "USD"}])
    calls = {"gateway": 0, "carrier": 0}

    def conn(sql, params):
        if sql.startswith("SELECT * FROM products"):
            return [dict(product)] if params[0] == product["sku"] else []
        if sql.startswith("SELECT stock"):
            return [{"stock": stock.get(params[0], 0)}]
        if sql.startswith("UPDATE stock"):
            stock[params[1]] -= params[0]
        return []

    def transport(service, method, path, payload, timeout_ms):
        if service in calls:
            seq = gateway if service == "gateway" else carrier
            body = seq[min(calls[service], len(seq) - 1)]
            calls[service] += 1
            return 200, body
        return 202, {"queued": True}

    db.connection = conn
    clients.transport = transport
    from shop import catalog
    catalog.clear_cache()
    return calls


def checkout(qty=1, country="DE", currency="EUR"):
    from shop import app
    return app.handle("POST", "/checkout", {
        "items": [{"sku": PRODUCT["sku"], "qty": qty}], "email": "a@example.com",
        "country": country, "currency": currency, "card_token": "tok_x"})
