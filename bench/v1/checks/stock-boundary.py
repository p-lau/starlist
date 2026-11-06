import unittest

from shop import db, inventory


class Check(unittest.TestCase):
    def test_can_buy_last_units(self):
        db.STOCK[1005] = 2
        inventory.reserve([{"sku": 1005, "qty": 2}])
        self.assertEqual(db.STOCK[1005], 0)

    def test_still_refuses_more_than_stock(self):
        db.STOCK[1006] = 1
        with self.assertRaises(inventory.OutOfStock):
            inventory.reserve([{"sku": 1006, "qty": 2}])
