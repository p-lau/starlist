import unittest
from decimal import Decimal

from shop import pricing


class PricingTest(unittest.TestCase):
    def test_subtotal(self):
        lines = [{"price": Decimal("2.50"), "qty": 2}, {"price": Decimal("1.00"), "qty": 1}]
        self.assertEqual(pricing.subtotal(lines), Decimal("6.00"))

    def test_tax(self):
        self.assertEqual(pricing.add_tax(Decimal("10.00"), "DE"), Decimal("11.90"))
