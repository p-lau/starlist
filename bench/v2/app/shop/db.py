"""Database access. The connection is installed at startup by the server process."""

from . import clock
from .config import SETTINGS
from .log import get_logger

log = get_logger("shop.db")

# Callable(sql, params) -> list of row dicts. Set at startup.
connection = None
query_count = 0


def query(sql: str, *params):
    global query_count
    query_count += 1
    started = clock.now()
    rows = connection(sql, params)
    ms = (clock.now() - started) * 1000
    log.debug("query %.1fms: %s params=%s", ms, sql, params)
    if ms >= SETTINGS["SLOW_QUERY_MS"]:
        log.warning("slow query %dms: %s", ms, sql)
    return rows
