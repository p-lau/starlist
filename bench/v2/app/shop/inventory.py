from . import db
from .log import get_logger

log = get_logger("shop.inventory")


class OutOfStock(Exception):
    def __init__(self, sku: int, requested: int, available: int):
        super().__init__(f"sku={sku} requested={requested} available={available}")
        self.sku = sku


def available(sku: int) -> int:
    return db.query("SELECT stock FROM stock WHERE sku = ?", sku)[0]["stock"]


def can_fulfil(stock: int, qty: int) -> bool:
    return stock >= qty


def reserve(lines) -> None:
    """Take stock for every line, or nothing at all."""
    for line in lines:
        stock = available(line["sku"])
        if not can_fulfil(stock, line["qty"]):
            log.info("insufficient stock sku=%d requested=%d available=%d",
                     line["sku"], line["qty"], stock)
            raise OutOfStock(line["sku"], line["qty"], stock)
    for line in lines:
        db.query("UPDATE stock SET stock = stock - ? WHERE sku = ?", line["qty"], line["sku"])
    log.info("reserved %d lines", len(lines))
