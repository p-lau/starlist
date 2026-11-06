"""Request entry point: routes a request and writes the access log line."""

import uuid

from . import catalog, clock, orders, rand
from .log import get_logger, request_id

log = get_logger("shop.http")


def handle(method: str, path: str, body: dict | None = None,
           user_agent: str = "-") -> tuple[int, dict]:
    rid = uuid.UUID(int=rand.rng.getrandbits(128)).hex[:16]
    token = request_id.set(rid)
    start = clock.now()
    log.debug("%s %s user_agent=%s", method, path, user_agent)
    try:
        status, payload = _route(method, path, body)
    except orders.CheckoutError as e:
        log.warning("checkout rejected: %s", e)
        status, payload = e.status, {"error": str(e)}
    except Exception:
        log.exception("unhandled error on %s %s", method, path)
        status, payload = 500, {"error": "internal"}
    ms = (clock.now() - start) * 1000
    log.info("%s %s %d %dms ua=%s", method, path, status, ms, user_agent.split(" ")[0])
    request_id.reset(token)
    return status, payload


def _route(method: str, path: str, body: dict | None):
    if path == "/healthz":
        return 200, {"ok": True}
    if method == "GET" and path.startswith("/products/"):
        try:
            return 200, catalog.get_product(int(path.rsplit("/", 1)[1]))
        except catalog.NotFound:
            return 404, {"error": "not found"}
    if method == "POST" and path == "/checkout":
        return 200, orders.checkout(body)
    return 404, {"error": "not found"}
