import unittest

import fake_world


class Check(unittest.TestCase):
    def test_quantity_sent_as_a_string(self):
        fake_world.install()
        status, body = fake_world.checkout(qty="2", country="US", currency="USD")
        self.assertEqual(status, 200, body)
        # 2 x 10.00 + 10.70 shipping, plus 7% tax
        self.assertEqual(body["total"], "32.85")

    def test_quantity_as_a_number(self):
        fake_world.install()
        status, body = fake_world.checkout(qty=2, country="US", currency="USD")
        self.assertEqual(status, 200, body)
        self.assertEqual(body["total"], "32.85")
