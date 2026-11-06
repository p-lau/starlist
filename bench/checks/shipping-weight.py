import unittest
from decimal import Decimal

from shop import shipping
from shop.config import SETTINGS


class Check(unittest.TestCase):
    def test_carrier_quote_used(self):
        lines = [{"weight_g": 500, "qty": 2}]
        price = shipping.quote(lines, "DE", Decimal("20.00"))
        self.assertNotEqual(price, SETTINGS["FLAT_SHIPPING"])
        # 9.50 base + 1.20/kg, about 1 kg plus packaging if any
        self.assertTrue(Decimal("10.60") <= price <= Decimal("11.00"), price)
