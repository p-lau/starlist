import unittest

import fake_world
from shop import payments


class Check(unittest.TestCase):
    def test_rate_limited_charge_is_retried(self):
        calls = fake_world.install(gateway=[
            {"status": "failed", "code": "rate_limited", "retry_after_ms": 500},
            {"status": "succeeded", "id": "ch_ok"}])
        status, body = fake_world.checkout()
        self.assertEqual(status, 200, body)
        self.assertEqual(calls["gateway"], 2)

    def test_real_declines_are_still_declines(self):
        fake_world.install(gateway=[{"status": "failed", "code": "insufficient_funds"}])
        status, _ = fake_world.checkout()
        self.assertEqual(status, 402)
