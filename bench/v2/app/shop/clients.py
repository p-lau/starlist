"""HTTP clients for the services the shop depends on: the payment gateway, the carrier's
rate API and the mail relay. The transport is installed at startup by the server process."""

from . import clock
from .log import get_logger

log = get_logger("shop.clients")

# Callable(service, method, path, payload) -> (status, body dict). Set at startup.
transport = None


class Timeout(Exception):
    pass


class UpstreamError(Exception):
    def __init__(self, service: str, status: int, body: dict):
        super().__init__(f"{service} returned HTTP {status}: {body}")
        self.status = status
        self.body = body


def call(service: str, method: str, path: str, payload: dict | None = None,
         timeout_ms: int = 10000) -> dict:
    started = clock.now()
    status, body = transport(service, method, path, payload, timeout_ms)
    ms = (clock.now() - started) * 1000
    log.debug("%s %s %s -> %d in %dms", service, method, path, status, ms)
    if status == 0:
        raise Timeout(f"{service} {method} {path}: no response after {timeout_ms}ms")
    if status >= 500:
        raise UpstreamError(service, status, body)
    return body
