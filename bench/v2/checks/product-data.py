import unittest

import fake_world


class Check(unittest.TestCase):
    def test_checkout_works_for_a_product_without_weight(self):
        fake_world.install(product={**fake_world.PRODUCT, "weight_g": None})
        status, body = fake_world.checkout()
        self.assertEqual(status, 200, body)

    def test_normal_products_still_get_carrier_rates(self):
        fake_world.install()
        status, body = fake_world.checkout(country="US", currency="USD")
        self.assertEqual(status, 200, body)
        # 10.00 + 10.70 shipping, plus 7% tax
        self.assertEqual(body["total"], "22.15")
