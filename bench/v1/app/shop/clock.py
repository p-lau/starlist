"""Time source. The service reads time only through here so it can be simulated."""

import time

_offset = None


def now() -> float:
    return _offset if _offset is not None else time.time()


def set_now(ts: float) -> None:
    global _offset
    _offset = ts


def advance(seconds: float) -> None:
    global _offset
    _offset = now() + seconds
