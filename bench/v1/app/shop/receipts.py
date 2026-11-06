from . import rand
from .config import SETTINGS
from .log import get_logger

log = get_logger("shop.receipts")


class SMTPServerDisconnected(Exception):
    pass


def format_receipt(order: dict) -> str:
    lines = [f"Order {order['id']}"]
    for line in order["lines"]:
        lines.append(f"  {line['qty']} x {line['name']}  {line['price']}")
    lines.append(f"Shipping: {order['shipping']}")
    lines.append(f"Total: {order['total']} {order['currency']}")
    return "\n".join(lines)


def _smtp_send(to: str, body: str) -> None:
    if rand.rng.random() < 0.004:
        raise SMTPServerDisconnected(f"Connection unexpectedly closed: {SETTINGS['SMTP_HOST']}:587")


def send_receipt(order: dict) -> None:
    body = format_receipt(order)
    _smtp_send(order["email"], body)
    log.info("receipt sent order=%s", order["id"])
