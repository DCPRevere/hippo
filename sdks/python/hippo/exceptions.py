from __future__ import annotations


class HippoError(Exception):
    """Base exception for Hippo SDK errors."""

    def __init__(self, message: str, status_code: int | None = None, body: str | None = None) -> None:
        self.status_code = status_code
        self.body = body
        super().__init__(message)


class AuthenticationError(HippoError):
    """Raised on HTTP 401 — invalid or missing API key."""


class ForbiddenError(HippoError):
    """Raised on HTTP 403 — insufficient permissions."""


class RateLimitError(HippoError):
    """Raised on HTTP 429 — too many requests."""

    def __init__(
        self,
        message: str,
        status_code: int | None = None,
        body: str | None = None,
        retry_after: float | None = None,
    ) -> None:
        super().__init__(message, status_code=status_code, body=body)
        self.retry_after = retry_after
