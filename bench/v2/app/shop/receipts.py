from . import clients
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


def send_receipt(order: dict) -> None:
    body = format_receipt(order)
    try:
        clients.call("mail", "POST", "/send", {"to": order["email"], "subject": "Your order",
                                               "body": body})
    except clients.UpstreamError as e:
        raise SMTPServerDisconnected(str(e))
    log.info("receipt sent order=%s", order["id"])
