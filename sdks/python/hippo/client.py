from __future__ import annotations

import asyncio
import json
import logging
import os
import random
import time
from email.utils import parsedate_to_datetime
from typing import Any, AsyncIterator, Iterator

import httpx

from hippo.exceptions import (
    AuthenticationError,
    ForbiddenError,
    HippoError,
    RateLimitError,
)
from hippo.models import (
    AskResponse,
    ContextResponse,
    CreateKeyResponse,
    CreateUserResponse,
    GraphEvent,
    HealthResponse,
    ListKeysResponse,
    ListUsersResponse,
    RememberBatchResponse,
    RememberResponse,
)

logger = logging.getLogger("hippo")

_STATUS_MAP: dict[int, type[HippoError]] = {
    401: AuthenticationError,
    403: ForbiddenError,
    429: RateLimitError,
}

_RETRYABLE_STATUS_CODES = {429, 502, 503, 504}


def _parse_retry_after(value: str | None) -> float | None:
    """Parse a Retry-After header value (seconds or HTTP date)."""
    if value is None:
        return None
    try:
        return float(value)
    except ValueError:
        pass
    try:
        dt = parsedate_to_datetime(value)
        delay = (dt.timestamp() - time.time())
        return max(delay, 0.0)
    except Exception:
        return None


def _raise_for_status(response: httpx.Response) -> None:
    if response.is_success:
        return
    body = response.text
    exc_cls = _STATUS_MAP.get(response.status_code, HippoError)
    if exc_cls is RateLimitError:
        retry_after = _parse_retry_after(response.headers.get("Retry-After"))
        raise RateLimitError(
            f"HTTP {response.status_code}: {body}",
            status_code=response.status_code,
            body=body,
            retry_after=retry_after,
        )
    raise exc_cls(
        f"HTTP {response.status_code}: {body}",
        status_code=response.status_code,
        body=body,
    )


def _build_headers(api_key: str | None) -> dict[str, str]:
    headers: dict[str, str] = {}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    return headers


def _backoff_delay(attempt: int) -> float:
    """Exponential backoff: 0.5, 1, 2, 4... with jitter."""
    base = 0.5 * (2 ** attempt)
    return base + random.uniform(0, base * 0.5)


class HippoClient:
    """Synchronous client for the Hippo REST API."""

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float = 30.0,
        max_retries: int = 3,
        http_client: httpx.Client | None = None,
    ) -> None:
        self.base_url = (base_url or os.environ.get("HIPPO_URL", "")).rstrip("/")
        self.api_key = api_key or os.environ.get("HIPPO_API_KEY")
        self._timeout = timeout
        self._max_retries = max_retries
        self._owns_client = http_client is None
        if http_client is not None:
            self._client = http_client
        else:
            self._client = httpx.Client(
                base_url=self.base_url,
                headers=_build_headers(self.api_key),
                timeout=timeout,
            )

    # ── Context manager ─────────────────────────────────────────

    def close(self) -> None:
        if self._owns_client:
            self._client.close()

    def __enter__(self) -> HippoClient:
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()

    # ── Internal helpers ────────────────────────────────────────

    def _request(
        self,
        method: str,
        path: str,
        *,
        json_body: dict[str, Any] | None = None,
        timeout: float | None = None,
    ) -> httpx.Response:
        effective_timeout = timeout if timeout is not None else self._timeout
        last_exc: Exception | None = None
        attempts = self._max_retries + 1  # 1 initial + retries

        for attempt in range(attempts):
            try:
                logger.debug("%s %s (attempt %d)", method, path, attempt + 1)
                resp = self._client.request(
                    method,
                    path,
                    json=json_body,
                    timeout=effective_timeout,
                )
                logger.debug("%s %s -> %d", method, path, resp.status_code)

                if resp.status_code in _RETRYABLE_STATUS_CODES and attempt < attempts - 1:
                    retry_after = _parse_retry_after(resp.headers.get("Retry-After"))
                    if retry_after is not None:
                        delay = retry_after
                        logger.warning(
                            "Rate limited on %s %s, retrying after %.1fs (Retry-After)",
                            method, path, delay,
                        )
                    else:
                        delay = _backoff_delay(attempt)
                        logger.warning(
                            "Retryable %d on %s %s, backing off %.1fs",
                            resp.status_code, method, path, delay,
                        )
                    time.sleep(delay)
                    continue

                return resp
            except httpx.TimeoutException as exc:
                last_exc = exc
                if attempt < attempts - 1:
                    delay = _backoff_delay(attempt)
                    logger.warning(
                        "Timeout on %s %s, backing off %.1fs",
                        method, path, delay,
                    )
                    time.sleep(delay)
                    continue
                raise

        # Should not reach here, but if it does return the last response
        assert last_exc is not None
        raise last_exc

    def _post(self, path: str, body: dict[str, Any], *, timeout: float | None = None) -> dict[str, Any]:
        resp = self._request("POST", path, json_body=body, timeout=timeout)
        _raise_for_status(resp)
        return resp.json()

    def _get(self, path: str, *, timeout: float | None = None) -> dict[str, Any]:
        resp = self._request("GET", path, timeout=timeout)
        _raise_for_status(resp)
        return resp.json()

    def _delete(self, path: str, *, timeout: float | None = None) -> None:
        resp = self._request("DELETE", path, timeout=timeout)
        _raise_for_status(resp)

    # ── Core endpoints ──────────────────────────────────────────

    def remember(
        self,
        statement: str,
        *,
        source_agent: str | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
        timeout: float | None = None,
    ) -> RememberResponse:
        body: dict[str, Any] = {"statement": statement}
        if source_agent is not None:
            body["source_agent"] = source_agent
        if graph is not None:
            body["graph"] = graph
        if ttl_secs is not None:
            body["ttl_secs"] = ttl_secs
        return RememberResponse.model_validate(self._post("/remember", body, timeout=timeout))

    def remember_batch(
        self,
        statements: list[str],
        *,
        source_agent: str | None = None,
        parallel: bool | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
        timeout: float | None = None,
    ) -> RememberBatchResponse:
        body: dict[str, Any] = {"statements": statements}
        if source_agent is not None:
            body["source_agent"] = source_agent
        if parallel is not None:
            body["parallel"] = parallel
        if graph is not None:
            body["graph"] = graph
        if ttl_secs is not None:
            body["ttl_secs"] = ttl_secs
        return RememberBatchResponse.model_validate(self._post("/remember/batch", body, timeout=timeout))

    def context(
        self,
        query: str,
        *,
        limit: int | None = None,
        max_hops: int | None = None,
        graph: str | None = None,
        timeout: float | None = None,
    ) -> ContextResponse:
        body: dict[str, Any] = {"query": query}
        if limit is not None:
            body["limit"] = limit
        if max_hops is not None:
            body["max_hops"] = max_hops
        if graph is not None:
            body["graph"] = graph
        return ContextResponse.model_validate(self._post("/context", body, timeout=timeout))

    def ask(
        self,
        question: str,
        *,
        limit: int | None = None,
        graph: str | None = None,
        verbose: bool | None = None,
        timeout: float | None = None,
    ) -> AskResponse:
        body: dict[str, Any] = {"question": question}
        if limit is not None:
            body["limit"] = limit
        if graph is not None:
            body["graph"] = graph
        if verbose is not None:
            body["verbose"] = verbose
        return AskResponse.model_validate(self._post("/ask", body, timeout=timeout))

    # ── Admin endpoints ─────────────────────────────────────────

    def create_user(
        self,
        user_id: str,
        display_name: str,
        *,
        role: str | None = None,
        graphs: list[str] | None = None,
        timeout: float | None = None,
    ) -> CreateUserResponse:
        body: dict[str, Any] = {"user_id": user_id, "display_name": display_name}
        if role is not None:
            body["role"] = role
        if graphs is not None:
            body["graphs"] = graphs
        return CreateUserResponse.model_validate(self._post("/admin/users", body, timeout=timeout))

    def list_users(self, *, timeout: float | None = None) -> ListUsersResponse:
        return ListUsersResponse.model_validate(self._get("/admin/users", timeout=timeout))

    def delete_user(self, user_id: str, *, timeout: float | None = None) -> None:
        self._delete(f"/admin/users/{user_id}", timeout=timeout)

    def create_key(self, user_id: str, label: str, *, timeout: float | None = None) -> CreateKeyResponse:
        return CreateKeyResponse.model_validate(
            self._post(f"/admin/users/{user_id}/keys", {"label": label}, timeout=timeout)
        )

    def list_keys(self, user_id: str, *, timeout: float | None = None) -> ListKeysResponse:
        return ListKeysResponse.model_validate(
            self._get(f"/admin/users/{user_id}/keys", timeout=timeout)
        )

    def delete_key(self, user_id: str, label: str, *, timeout: float | None = None) -> None:
        self._delete(f"/admin/users/{user_id}/keys/{label}", timeout=timeout)

    # ── Observability ───────────────────────────────────────────

    def health(self, *, timeout: float | None = None) -> HealthResponse:
        return HealthResponse.model_validate(self._get("/health", timeout=timeout))

    # ── SSE streaming ───────────────────────────────────────────

    def events(self, *, graph: str | None = None, timeout: float | None = None) -> Iterator[GraphEvent]:
        """Stream Server-Sent Events from GET /events."""
        params: dict[str, str] = {}
        if graph is not None:
            params["graph"] = graph
        effective_timeout = timeout if timeout is not None else self._timeout
        with self._client.stream(
            "GET",
            "/events",
            params=params,
            timeout=effective_timeout,
        ) as resp:
            _raise_for_status(resp)
            event_type: str | None = None
            data_lines: list[str] = []
            for line in resp.iter_lines():
                if line.startswith("event:"):
                    event_type = line[len("event:"):].strip()
                elif line.startswith("data:"):
                    data_lines.append(line[len("data:"):].strip())
                elif line == "":
                    # Empty line = end of event
                    if event_type is not None and data_lines:
                        raw_data = "\n".join(data_lines)
                        try:
                            payload = json.loads(raw_data)
                        except json.JSONDecodeError:
                            payload = {"raw": raw_data}
                        yield GraphEvent(event=event_type, data=payload)
                    event_type = None
                    data_lines = []


class AsyncHippoClient:
    """Asynchronous client for the Hippo REST API."""

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float = 30.0,
        max_retries: int = 3,
        http_client: httpx.AsyncClient | None = None,
    ) -> None:
        self.base_url = (base_url or os.environ.get("HIPPO_URL", "")).rstrip("/")
        self.api_key = api_key or os.environ.get("HIPPO_API_KEY")
        self._timeout = timeout
        self._max_retries = max_retries
        self._owns_client = http_client is None
        if http_client is not None:
            self._client = http_client
        else:
            self._client = httpx.AsyncClient(
                base_url=self.base_url,
                headers=_build_headers(self.api_key),
                timeout=timeout,
            )

    # ── Context manager ─────────────────────────────────────────

    async def close(self) -> None:
        if self._owns_client:
            await self._client.aclose()

    async def __aenter__(self) -> AsyncHippoClient:
        return self

    async def __aexit__(self, *args: Any) -> None:
        await self.close()

    # ── Internal helpers ────────────────────────────────────────

    async def _request(
        self,
        method: str,
        path: str,
        *,
        json_body: dict[str, Any] | None = None,
        timeout: float | None = None,
    ) -> httpx.Response:
        effective_timeout = timeout if timeout is not None else self._timeout
        last_exc: Exception | None = None
        attempts = self._max_retries + 1

        for attempt in range(attempts):
            try:
                logger.debug("%s %s (attempt %d)", method, path, attempt + 1)
                resp = await self._client.request(
                    method,
                    path,
                    json=json_body,
                    timeout=effective_timeout,
                )
                logger.debug("%s %s -> %d", method, path, resp.status_code)

                if resp.status_code in _RETRYABLE_STATUS_CODES and attempt < attempts - 1:
                    retry_after = _parse_retry_after(resp.headers.get("Retry-After"))
                    if retry_after is not None:
                        delay = retry_after
                        logger.warning(
                            "Rate limited on %s %s, retrying after %.1fs (Retry-After)",
                            method, path, delay,
                        )
                    else:
                        delay = _backoff_delay(attempt)
                        logger.warning(
                            "Retryable %d on %s %s, backing off %.1fs",
                            resp.status_code, method, path, delay,
                        )
                    await asyncio.sleep(delay)
                    continue

                return resp
            except httpx.TimeoutException as exc:
                last_exc = exc
                if attempt < attempts - 1:
                    delay = _backoff_delay(attempt)
                    logger.warning(
                        "Timeout on %s %s, backing off %.1fs",
                        method, path, delay,
                    )
                    await asyncio.sleep(delay)
                    continue
                raise

        assert last_exc is not None
        raise last_exc

    async def _post(self, path: str, body: dict[str, Any], *, timeout: float | None = None) -> dict[str, Any]:
        resp = await self._request("POST", path, json_body=body, timeout=timeout)
        _raise_for_status(resp)
        return resp.json()

    async def _get(self, path: str, *, timeout: float | None = None) -> dict[str, Any]:
        resp = await self._request("GET", path, timeout=timeout)
        _raise_for_status(resp)
        return resp.json()

    async def _delete(self, path: str, *, timeout: float | None = None) -> None:
        resp = await self._request("DELETE", path, timeout=timeout)
        _raise_for_status(resp)

    # ── Core endpoints ──────────────────────────────────────────

    async def remember(
        self,
        statement: str,
        *,
        source_agent: str | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
        timeout: float | None = None,
    ) -> RememberResponse:
        body: dict[str, Any] = {"statement": statement}
        if source_agent is not None:
            body["source_agent"] = source_agent
        if graph is not None:
            body["graph"] = graph
        if ttl_secs is not None:
            body["ttl_secs"] = ttl_secs
        return RememberResponse.model_validate(await self._post("/remember", body, timeout=timeout))

    async def remember_batch(
        self,
        statements: list[str],
        *,
        source_agent: str | None = None,
        parallel: bool | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
        timeout: float | None = None,
    ) -> RememberBatchResponse:
        body: dict[str, Any] = {"statements": statements}
        if source_agent is not None:
            body["source_agent"] = source_agent
        if parallel is not None:
            body["parallel"] = parallel
        if graph is not None:
            body["graph"] = graph
        if ttl_secs is not None:
            body["ttl_secs"] = ttl_secs
        return RememberBatchResponse.model_validate(await self._post("/remember/batch", body, timeout=timeout))

    async def context(
        self,
        query: str,
        *,
        limit: int | None = None,
        max_hops: int | None = None,
        graph: str | None = None,
        timeout: float | None = None,
    ) -> ContextResponse:
        body: dict[str, Any] = {"query": query}
        if limit is not None:
            body["limit"] = limit
        if max_hops is not None:
            body["max_hops"] = max_hops
        if graph is not None:
            body["graph"] = graph
        return ContextResponse.model_validate(await self._post("/context", body, timeout=timeout))

    async def ask(
        self,
        question: str,
        *,
        limit: int | None = None,
        graph: str | None = None,
        verbose: bool | None = None,
        timeout: float | None = None,
    ) -> AskResponse:
        body: dict[str, Any] = {"question": question}
        if limit is not None:
            body["limit"] = limit
        if graph is not None:
            body["graph"] = graph
        if verbose is not None:
            body["verbose"] = verbose
        return AskResponse.model_validate(await self._post("/ask", body, timeout=timeout))

    # ── Admin endpoints ─────────────────────────────────────────

    async def create_user(
        self,
        user_id: str,
        display_name: str,
        *,
        role: str | None = None,
        graphs: list[str] | None = None,
        timeout: float | None = None,
    ) -> CreateUserResponse:
        body: dict[str, Any] = {"user_id": user_id, "display_name": display_name}
        if role is not None:
            body["role"] = role
        if graphs is not None:
            body["graphs"] = graphs
        return CreateUserResponse.model_validate(await self._post("/admin/users", body, timeout=timeout))

    async def list_users(self, *, timeout: float | None = None) -> ListUsersResponse:
        return ListUsersResponse.model_validate(await self._get("/admin/users", timeout=timeout))

    async def delete_user(self, user_id: str, *, timeout: float | None = None) -> None:
        await self._delete(f"/admin/users/{user_id}", timeout=timeout)

    async def create_key(self, user_id: str, label: str, *, timeout: float | None = None) -> CreateKeyResponse:
        return CreateKeyResponse.model_validate(
            await self._post(f"/admin/users/{user_id}/keys", {"label": label}, timeout=timeout)
        )

    async def list_keys(self, user_id: str, *, timeout: float | None = None) -> ListKeysResponse:
        return ListKeysResponse.model_validate(
            await self._get(f"/admin/users/{user_id}/keys", timeout=timeout)
        )

    async def delete_key(self, user_id: str, label: str, *, timeout: float | None = None) -> None:
        await self._delete(f"/admin/users/{user_id}/keys/{label}", timeout=timeout)

    # ── Observability ───────────────────────────────────────────

    async def health(self, *, timeout: float | None = None) -> HealthResponse:
        return HealthResponse.model_validate(await self._get("/health", timeout=timeout))

    # ── SSE streaming ───────────────────────────────────────────

    async def events(self, *, graph: str | None = None, timeout: float | None = None) -> AsyncIterator[GraphEvent]:
        """Stream Server-Sent Events from GET /events."""
        params: dict[str, str] = {}
        if graph is not None:
            params["graph"] = graph
        effective_timeout = timeout if timeout is not None else self._timeout
        async with self._client.stream(
            "GET",
            "/events",
            params=params,
            timeout=effective_timeout,
        ) as resp:
            _raise_for_status(resp)
            event_type: str | None = None
            data_lines: list[str] = []
            async for line in resp.aiter_lines():
                if line.startswith("event:"):
                    event_type = line[len("event:"):].strip()
                elif line.startswith("data:"):
                    data_lines.append(line[len("data:"):].strip())
                elif line == "":
                    if event_type is not None and data_lines:
                        raw_data = "\n".join(data_lines)
                        try:
                            payload = json.loads(raw_data)
                        except json.JSONDecodeError:
                            payload = {"raw": raw_data}
                        yield GraphEvent(event=event_type, data=payload)
                    event_type = None
                    data_lines = []
