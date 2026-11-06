"""Shared random source for simulated latency and upstream failures."""

import random

rng = random.Random(7)


def seed(value: int) -> None:
    rng.seed(value)
