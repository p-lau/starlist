import unittest
from decimal import Decimal

from shop import receipts


class Check(unittest.TestCase):
    def test_receipt_with_gift_card(self):
        order = {"id": "ord_1", "lines": [{"qty": 1, "name": "mug", "price": Decimal("9.99")}],
                 "shipping": Decimal("4.99"), "total": Decimal("4.98"), "currency": "USD",
                 "gift_card": {"code": "GC-7781-2231", "amount": Decimal("10.00")}}
        text = receipts.format_receipt(order)
        self.assertIn("10.00", text)
        self.assertIn("2231", text)

    def test_receipt_without_gift_card(self):
        order = {"id": "ord_2", "lines": [], "shipping": Decimal("0"), "total": Decimal("1.00"),
                 "currency": "USD"}
        self.assertIn("Total: 1.00 USD", receipts.format_receipt(order))
