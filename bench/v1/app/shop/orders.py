from decimal import Decimal

from . import catalog, inventory, payments, pricing, rand, receipts, shipping
from .log import get_logger

log = get_logger("shop.orders")


class CheckoutError(Exception):
    def __init__(self, status: int, message: str):
        super().__init__(message)
        self.status = status


def _new_order_id() -> str:
    return "ord_" + "".join(rand.rng.choice("0123456789abcdef") for _ in range(10))


def checkout(req: dict) -> dict:
    lines = []
    for item in req["items"]:
        product = catalog.get_product(item["sku"])
        lines.append({**product, "qty": item["qty"]})
    log.info("checkout user=%s items=%d country=%s coupon=%s",
             req["email"], len(lines), req["country"], req.get("coupon") or "-")
    try:
        items_total = pricing.apply_coupon(pricing.subtotal(lines), req.get("coupon"))
    except pricing.InvalidCoupon as e:
        raise CheckoutError(400, f"invalid coupon {e}")
    ship = shipping.quote(lines, req["country"], items_total)
    total_usd = pricing.add_tax(items_total + ship, req["country"])
    total = pricing.convert(total_usd, req["currency"])
    try:
        inventory.reserve(lines)
    except inventory.OutOfStock as e:
        raise CheckoutError(409, f"out of stock: sku={e.sku}")
    try:
        charge_id = payments.charge(total, req["currency"], req["card_token"])
    except payments.PaymentDeclined as e:
        raise CheckoutError(402, str(e))
    order = {"id": _new_order_id(), "email": req["email"], "lines": lines,
             "shipping": ship, "total": total, "currency": req["currency"],
             "charge": charge_id}
    log.info("order placed order=%s total=%s %s", order["id"], total, req["currency"])
    try:
        receipts.send_receipt(order)
    except Exception:
        log.exception("failed to send receipt order=%s", order["id"])
    return {"order": order["id"], "total": str(total)}
