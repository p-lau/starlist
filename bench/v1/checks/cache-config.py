import unittest

from shop import catalog, clock, db


class Check(unittest.TestCase):
    def test_second_lookup_is_cached(self):
        clock.set_now(1_700_000_000)
        catalog.clear_cache()
        catalog.get_product(1001)
        before = db.query_count
        clock.advance(30)
        catalog.get_product(1001)
        self.assertEqual(db.query_count, before, "second lookup within 30s hit the database")
