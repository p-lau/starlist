import unittest
from decimal import Decimal

import fake_world
from shop import shipping


class Check(unittest.TestCase):
    lines = [{"weight_g": 500, "qty": 1}]

    def test_new_rate_format(self):
        fake_world.install(carrier=[{"rate": {"amount_cents": 1070, "currency": "USD"},
                                     "service": "standard", "api_version": "2026-10"}])
        self.assertEqual(shipping.quote(self.lines, "DE", Decimal("20")), Decimal("10.70"))
