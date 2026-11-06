import unittest
from decimal import Decimal

from shop import pricing


class Check(unittest.TestCase):
    def test_coupon_takes_percent_off(self):
        self.assertEqual(pricing.apply_coupon(Decimal("100.00"), "SAVE15"), Decimal("85.00"))
        self.assertEqual(pricing.apply_coupon(Decimal("50.00"), "VIP20"), Decimal("40.00"))

    def test_total_with_coupon_is_positive(self):
        lines = [{"price": Decimal("20.00"), "qty": 2}]
        total = pricing.order_total(lines, "SAVE10", Decimal("5.00"), "US")
        self.assertEqual(total, Decimal("43.87"))
