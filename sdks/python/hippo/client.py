from __future__ import annotations

import asyncio
import json
import logging
import os
import random
import time
from email.utils import parsedate_to_datetime
from typing import Any, AsyncIterator, Iterator
from urllib.parse import urlencode

import httpx

from hippo.exceptions import (
    AuthenticationError,
    ForbiddenError,
    HippoError,
    RateLimitError,
)
from hippo.models import (
    AskRequest,
    AskResponse,
    AuditResponse,
    ContextRequest,
    ContextResponse,
    CorrectRequest,
    CorrectResponse,
    CreateKeyResponse,
    CreateUserResponse,
    GraphEvent,
    GraphsListResponse,
    HealthResponse,
    ListKeysResponse,
    ListUsersResponse,
    RememberBatchResponse,
    RememberRequest,
    RememberResponse,
    RetractRequest,
    RetractResponse,
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


# `/health` is the only endpoint exposed at the server root; everything else
# lives under `/api`. We prepend it transparently so callers can pass plain
# paths like `/remember`.
_ROOT_PATHS = {"/health"}


def _api_path(path: str) -> str:
    if path.startswith("/api/") or path == "/api" or path in _ROOT_PATHS:
        return path
    return f"/api{path}"


def _qs(params: dict[str, Any]) -> str:
    pairs = [(k, v) for k, v in params.items() if v is not None]
    return urlencode(pairs)


def _drop_none(body: dict[str, Any]) -> dict[str, Any]:
    return {k: v for k, v in body.items() if v is not None}


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
        full_path = _api_path(path)
        effective_timeout = timeout if timeout is not None else self._timeout
        last_exc: Exception | None = None
        attempts = self._max_retries + 1  # 1 initial + retries

        for attempt in range(attempts):
            try:
                logger.debug("%s %s (attempt %d)", method, full_path, attempt + 1)
                resp = self._client.request(
                    method,
                    full_path,
                    json=json_body,
                    timeout=effective_timeout,
                )
                logger.debug("%s %s -> %d", method, full_path, resp.status_code)

                if resp.status_code in _RETRYABLE_STATUS_CODES and attempt < attempts - 1:
                    retry_after = _parse_retry_after(resp.headers.get("Retry-After"))
                    if retry_after is not None:
                        delay = retry_after
                        logger.warning(
                            "Rate limited on %s %s, retrying after %.1fs (Retry-After)",
                            method, full_path, delay,
                        )
                    else:
                        delay = _backoff_delay(attempt)
                        logger.warning(
                            "Retryable %d on %s %s, backing off %.1fs",
                            resp.status_code, method, full_path, delay,
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
                        method, full_path, delay,
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

    def _get(self, path: str, *, timeout: float | None = None) -> Any:
        resp = self._request("GET", path, timeout=timeout)
        _raise_for_status(resp)
        return resp.json()

    def _get_text(self, path: str, *, timeout: float | None = None) -> str:
        resp = self._request("GET", path, timeout=timeout)
        _raise_for_status(resp)
        return resp.text

    def _delete(self, path: str, *, timeout: float | None = None) -> Any:
        resp = self._request("DELETE", path, timeout=timeout)
        _raise_for_status(resp)
        if not resp.content:
            return None
        try:
            return resp.json()
        except Exception:
            return None

    # ── Core endpoints ──────────────────────────────────────────

    def remember(
        self,
        statement: str,
        *,
        source_agent: str | None = None,
        source_credibility_hint: float | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
        timeout: float | None = None,
    ) -> RememberResponse:
        body = RememberRequest(
            statement=statement,
            source_agent=source_agent,
            source_credibility_hint=source_credibility_hint,
            graph=graph,
            ttl_secs=ttl_secs,
        ).model_dump(exclude_none=True)
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
        memory_tier_filter: str | None = None,
        graph: str | None = None,
        at: str | None = None,
        scoring: dict[str, Any] | None = None,
        timeout: float | None = None,
    ) -> ContextResponse:
        body = _drop_none({
            "query": query,
            "limit": limit,
            "max_hops": max_hops,
            "memory_tier_filter": memory_tier_filter,
            "graph": graph,
            "at": at,
            "scoring": scoring,
        })
        return ContextResponse.model_validate(self._post("/context", body, timeout=timeout))

    def ask(
        self,
        question: str,
        *,
        limit: int | None = None,
        graph: str | None = None,
        verbose: bool | None = None,
        max_iterations: int | None = None,
        timeout: float | None = None,
    ) -> AskResponse:
        body = _drop_none({
            "question": question,
            "limit": limit,
            "graph": graph,
            "verbose": verbose,
            "max_iterations": max_iterations,
        })
        return AskResponse.model_validate(self._post("/ask", body, timeout=timeout))

    # ── REST resources ──────────────────────────────────────────

    def get_entity(self, entity_id: str, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/entities/{entity_id}"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return self._get(path, timeout=timeout)

    def delete_entity(self, entity_id: str, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/entities/{entity_id}"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        result = self._delete(path, timeout=timeout)
        return result or {}

    def entity_edges(self, entity_id: str, *, graph: str | None = None, timeout: float | None = None) -> list[dict[str, Any]]:
        path = f"/entities/{entity_id}/edges"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return self._get(path, timeout=timeout)

    def get_edge(self, edge_id: int, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/edges/{edge_id}"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return self._get(path, timeout=timeout)

    def edge_provenance(self, edge_id: int, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/edges/{edge_id}/provenance"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return self._get(path, timeout=timeout)

    # ── Destructive ops ─────────────────────────────────────────

    def retract(
        self,
        edge_id: int,
        *,
        reason: str | None = None,
        graph: str | None = None,
        timeout: float | None = None,
    ) -> RetractResponse:
        body = RetractRequest(edge_id=edge_id, reason=reason, graph=graph).model_dump(exclude_none=True)
        return RetractResponse.model_validate(self._post("/retract", body, timeout=timeout))

    def correct(
        self,
        edge_id: int,
        statement: str,
        *,
        reason: str | None = None,
        source_agent: str | None = None,
        graph: str | None = None,
        timeout: float | None = None,
    ) -> CorrectResponse:
        body = CorrectRequest(
            edge_id=edge_id,
            statement=statement,
            reason=reason,
            source_agent=source_agent,
            graph=graph,
        ).model_dump(exclude_none=True)
        return CorrectResponse.model_validate(self._post("/correct", body, timeout=timeout))

    # ── Operations ──────────────────────────────────────────────

    def maintain(self, *, timeout: float | None = None) -> dict[str, Any]:
        return self._post("/maintain", {}, timeout=timeout)

    def graph(
        self,
        *,
        graph: str | None = None,
        format: str | None = None,
        timeout: float | None = None,
    ) -> Any:
        params = _qs({"graph": graph, "format": format})
        path = "/graph" + (f"?{params}" if params else "")
        if format in ("graphml", "csv"):
            return self._get_text(path, timeout=timeout)
        return self._get(path, timeout=timeout)

    # ── Graphs ──────────────────────────────────────────────────

    def list_graphs(self, *, timeout: float | None = None) -> GraphsListResponse:
        return GraphsListResponse.model_validate(self._get("/graphs", timeout=timeout))

    def drop_graph(self, name: str, *, timeout: float | None = None) -> dict[str, Any]:
        result = self._delete(f"/graphs/drop/{name}", timeout=timeout)
        return result or {}

    def seed(self, payload: dict[str, Any], *, timeout: float | None = None) -> dict[str, Any]:
        return self._post("/seed", payload, timeout=timeout)

    def backup(self, *, graph: str | None = None, timeout: float | None = None) -> str:
        body = _drop_none({"graph": graph})
        resp = self._request("POST", "/admin/backup", json_body=body, timeout=timeout)
        _raise_for_status(resp)
        return resp.text

    def restore(self, payload: dict[str, Any], *, timeout: float | None = None) -> dict[str, Any]:
        return self._post("/admin/restore", payload, timeout=timeout)

    def openapi(self, *, timeout: float | None = None) -> str:
        return self._get_text("/openapi.yaml", timeout=timeout)

    def metrics(self, *, timeout: float | None = None) -> str:
        return self._get_text("/metrics", timeout=timeout)

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

    def audit(
        self,
        *,
        user_id: str | None = None,
        action: str | None = None,
        limit: int | None = None,
        timeout: float | None = None,
    ) -> AuditResponse:
        params = _qs({"user_id": user_id, "action": action, "limit": limit})
        path = "/admin/audit" + (f"?{params}" if params else "")
        return AuditResponse.model_validate(self._get(path, timeout=timeout))

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
            _api_path("/events"),
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
        full_path = _api_path(path)
        effective_timeout = timeout if timeout is not None else self._timeout
        last_exc: Exception | None = None
        attempts = self._max_retries + 1

        for attempt in range(attempts):
            try:
                logger.debug("%s %s (attempt %d)", method, full_path, attempt + 1)
                resp = await self._client.request(
                    method,
                    full_path,
                    json=json_body,
                    timeout=effective_timeout,
                )
                logger.debug("%s %s -> %d", method, full_path, resp.status_code)

                if resp.status_code in _RETRYABLE_STATUS_CODES and attempt < attempts - 1:
                    retry_after = _parse_retry_after(resp.headers.get("Retry-After"))
                    if retry_after is not None:
                        delay = retry_after
                        logger.warning(
                            "Rate limited on %s %s, retrying after %.1fs (Retry-After)",
                            method, full_path, delay,
                        )
                    else:
                        delay = _backoff_delay(attempt)
                        logger.warning(
                            "Retryable %d on %s %s, backing off %.1fs",
                            resp.status_code, method, full_path, delay,
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
                        method, full_path, delay,
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

    async def _get(self, path: str, *, timeout: float | None = None) -> Any:
        resp = await self._request("GET", path, timeout=timeout)
        _raise_for_status(resp)
        return resp.json()

    async def _get_text(self, path: str, *, timeout: float | None = None) -> str:
        resp = await self._request("GET", path, timeout=timeout)
        _raise_for_status(resp)
        return resp.text

    async def _delete(self, path: str, *, timeout: float | None = None) -> Any:
        resp = await self._request("DELETE", path, timeout=timeout)
        _raise_for_status(resp)
        if not resp.content:
            return None
        try:
            return resp.json()
        except Exception:
            return None

    # ── Core endpoints ──────────────────────────────────────────

    async def remember(
        self,
        statement: str,
        *,
        source_agent: str | None = None,
        source_credibility_hint: float | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
        timeout: float | None = None,
    ) -> RememberResponse:
        body = RememberRequest(
            statement=statement,
            source_agent=source_agent,
            source_credibility_hint=source_credibility_hint,
            graph=graph,
            ttl_secs=ttl_secs,
        ).model_dump(exclude_none=True)
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
        memory_tier_filter: str | None = None,
        graph: str | None = None,
        at: str | None = None,
        scoring: dict[str, Any] | None = None,
        timeout: float | None = None,
    ) -> ContextResponse:
        body = _drop_none({
            "query": query,
            "limit": limit,
            "max_hops": max_hops,
            "memory_tier_filter": memory_tier_filter,
            "graph": graph,
            "at": at,
            "scoring": scoring,
        })
        return ContextResponse.model_validate(await self._post("/context", body, timeout=timeout))

    async def ask(
        self,
        question: str,
        *,
        limit: int | None = None,
        graph: str | None = None,
        verbose: bool | None = None,
        max_iterations: int | None = None,
        timeout: float | None = None,
    ) -> AskResponse:
        body = _drop_none({
            "question": question,
            "limit": limit,
            "graph": graph,
            "verbose": verbose,
            "max_iterations": max_iterations,
        })
        return AskResponse.model_validate(await self._post("/ask", body, timeout=timeout))

    # ── REST resources ──────────────────────────────────────────

    async def get_entity(self, entity_id: str, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/entities/{entity_id}"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return await self._get(path, timeout=timeout)

    async def delete_entity(self, entity_id: str, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/entities/{entity_id}"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        result = await self._delete(path, timeout=timeout)
        return result or {}

    async def entity_edges(self, entity_id: str, *, graph: str | None = None, timeout: float | None = None) -> list[dict[str, Any]]:
        path = f"/entities/{entity_id}/edges"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return await self._get(path, timeout=timeout)

    async def get_edge(self, edge_id: int, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/edges/{edge_id}"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return await self._get(path, timeout=timeout)

    async def edge_provenance(self, edge_id: int, *, graph: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        path = f"/edges/{edge_id}/provenance"
        if graph:
            path += f"?{_qs({'graph': graph})}"
        return await self._get(path, timeout=timeout)

    # ── Destructive ops ─────────────────────────────────────────

    async def retract(
        self,
        edge_id: int,
        *,
        reason: str | None = None,
        graph: str | None = None,
        timeout: float | None = None,
    ) -> RetractResponse:
        body = RetractRequest(edge_id=edge_id, reason=reason, graph=graph).model_dump(exclude_none=True)
        return RetractResponse.model_validate(await self._post("/retract", body, timeout=timeout))

    async def correct(
        self,
        edge_id: int,
        statement: str,
        *,
        reason: str | None = None,
        source_agent: str | None = None,
        graph: str | None = None,
        timeout: float | None = None,
    ) -> CorrectResponse:
        body = CorrectRequest(
            edge_id=edge_id,
            statement=statement,
            reason=reason,
            source_agent=source_agent,
            graph=graph,
        ).model_dump(exclude_none=True)
        return CorrectResponse.model_validate(await self._post("/correct", body, timeout=timeout))

    # ── Operations ──────────────────────────────────────────────

    async def maintain(self, *, timeout: float | None = None) -> dict[str, Any]:
        return await self._post("/maintain", {}, timeout=timeout)

    async def graph(
        self,
        *,
        graph: str | None = None,
        format: str | None = None,
        timeout: float | None = None,
    ) -> Any:
        params = _qs({"graph": graph, "format": format})
        path = "/graph" + (f"?{params}" if params else "")
        if format in ("graphml", "csv"):
            return await self._get_text(path, timeout=timeout)
        return await self._get(path, timeout=timeout)

    # ── Graphs ──────────────────────────────────────────────────

    async def list_graphs(self, *, timeout: float | None = None) -> GraphsListResponse:
        return GraphsListResponse.model_validate(await self._get("/graphs", timeout=timeout))

    async def drop_graph(self, name: str, *, timeout: float | None = None) -> dict[str, Any]:
        result = await self._delete(f"/graphs/drop/{name}", timeout=timeout)
        return result or {}

    async def seed(self, payload: dict[str, Any], *, timeout: float | None = None) -> dict[str, Any]:
        return await self._post("/seed", payload, timeout=timeout)

    async def backup(self, *, graph: str | None = None, timeout: float | None = None) -> str:
        body = _drop_none({"graph": graph})
        resp = await self._request("POST", "/admin/backup", json_body=body, timeout=timeout)
        _raise_for_status(resp)
        return resp.text

    async def restore(self, payload: dict[str, Any], *, timeout: float | None = None) -> dict[str, Any]:
        return await self._post("/admin/restore", payload, timeout=timeout)

    async def openapi(self, *, timeout: float | None = None) -> str:
        return await self._get_text("/openapi.yaml", timeout=timeout)

    async def metrics(self, *, timeout: float | None = None) -> str:
        return await self._get_text("/metrics", timeout=timeout)

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

    async def audit(
        self,
        *,
        user_id: str | None = None,
        action: str | None = None,
        limit: int | None = None,
        timeout: float | None = None,
    ) -> AuditResponse:
        params = _qs({"user_id": user_id, "action": action, "limit": limit})
        path = "/admin/audit" + (f"?{params}" if params else "")
        return AuditResponse.model_validate(await self._get(path, timeout=timeout))

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
            _api_path("/events"),
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
