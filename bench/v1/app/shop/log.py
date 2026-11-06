import contextvars
import logging

request_id = contextvars.ContextVar("request_id", default="-")


class _RequestFilter(logging.Filter):
    def filter(self, record: logging.LogRecord) -> bool:
        record.request_id = request_id.get()
        return True


def get_logger(name: str) -> logging.Logger:
    logger = logging.getLogger(name)
    if not any(isinstance(f, _RequestFilter) for f in logger.filters):
        logger.addFilter(_RequestFilter())
    return logger
