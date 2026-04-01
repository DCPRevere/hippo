from __future__ import annotations

import os
from typing import Any

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
    HealthResponse,
    ListKeysResponse,
    ListUsersResponse,
    RememberBatchResponse,
    RememberResponse,
)

_STATUS_MAP: dict[int, type[HippoError]] = {
    401: AuthenticationError,
    403: ForbiddenError,
    429: RateLimitError,
}


def _raise_for_status(response: httpx.Response) -> None:
    if response.is_success:
        return
    body = response.text
    exc_cls = _STATUS_MAP.get(response.status_code, HippoError)
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


class HippoClient:
    """Synchronous client for the Hippo REST API."""

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float = 30.0,
    ) -> None:
        self.base_url = (base_url or os.environ.get("HIPPO_URL", "")).rstrip("/")
        self.api_key = api_key or os.environ.get("HIPPO_API_KEY")
        self._client = httpx.Client(
            base_url=self.base_url,
            headers=_build_headers(self.api_key),
            timeout=timeout,
        )

    # ── Context manager ─────────────────────────────────────────

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> HippoClient:
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()

    # ── Internal helpers ────────────────────────────────────────

    def _post(self, path: str, body: dict[str, Any]) -> dict[str, Any]:
        resp = self._client.post(path, json=body)
        _raise_for_status(resp)
        return resp.json()

    def _get(self, path: str) -> dict[str, Any]:
        resp = self._client.get(path)
        _raise_for_status(resp)
        return resp.json()

    def _delete(self, path: str) -> None:
        resp = self._client.delete(path)
        _raise_for_status(resp)

    # ── Core endpoints ──────────────────────────────────────────

    def remember(
        self,
        statement: str,
        *,
        source_agent: str | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
    ) -> RememberResponse:
        body: dict[str, Any] = {"statement": statement}
        if source_agent is not None:
            body["source_agent"] = source_agent
        if graph is not None:
            body["graph"] = graph
        if ttl_secs is not None:
            body["ttl_secs"] = ttl_secs
        return RememberResponse.model_validate(self._post("/remember", body))

    def remember_batch(
        self,
        statements: list[str],
        *,
        source_agent: str | None = None,
        parallel: bool | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
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
        return RememberBatchResponse.model_validate(self._post("/remember/batch", body))

    def context(
        self,
        query: str,
        *,
        limit: int | None = None,
        max_hops: int | None = None,
        graph: str | None = None,
    ) -> ContextResponse:
        body: dict[str, Any] = {"query": query}
        if limit is not None:
            body["limit"] = limit
        if max_hops is not None:
            body["max_hops"] = max_hops
        if graph is not None:
            body["graph"] = graph
        return ContextResponse.model_validate(self._post("/context", body))

    def ask(
        self,
        question: str,
        *,
        limit: int | None = None,
        graph: str | None = None,
        verbose: bool | None = None,
    ) -> AskResponse:
        body: dict[str, Any] = {"question": question}
        if limit is not None:
            body["limit"] = limit
        if graph is not None:
            body["graph"] = graph
        if verbose is not None:
            body["verbose"] = verbose
        return AskResponse.model_validate(self._post("/ask", body))

    # ── Admin endpoints ─────────────────────────────────────────

    def create_user(
        self,
        user_id: str,
        display_name: str,
        *,
        role: str | None = None,
        graphs: list[str] | None = None,
    ) -> CreateUserResponse:
        body: dict[str, Any] = {"user_id": user_id, "display_name": display_name}
        if role is not None:
            body["role"] = role
        if graphs is not None:
            body["graphs"] = graphs
        return CreateUserResponse.model_validate(self._post("/admin/users", body))

    def list_users(self) -> ListUsersResponse:
        return ListUsersResponse.model_validate(self._get("/admin/users"))

    def delete_user(self, user_id: str) -> None:
        self._delete(f"/admin/users/{user_id}")

    def create_key(self, user_id: str, label: str) -> CreateKeyResponse:
        return CreateKeyResponse.model_validate(
            self._post(f"/admin/users/{user_id}/keys", {"label": label})
        )

    def list_keys(self, user_id: str) -> ListKeysResponse:
        return ListKeysResponse.model_validate(
            self._get(f"/admin/users/{user_id}/keys")
        )

    def delete_key(self, user_id: str, label: str) -> None:
        self._delete(f"/admin/users/{user_id}/keys/{label}")

    # ── Observability ───────────────────────────────────────────

    def health(self) -> HealthResponse:
        return HealthResponse.model_validate(self._get("/health"))


class AsyncHippoClient:
    """Asynchronous client for the Hippo REST API."""

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float = 30.0,
    ) -> None:
        self.base_url = (base_url or os.environ.get("HIPPO_URL", "")).rstrip("/")
        self.api_key = api_key or os.environ.get("HIPPO_API_KEY")
        self._client = httpx.AsyncClient(
            base_url=self.base_url,
            headers=_build_headers(self.api_key),
            timeout=timeout,
        )

    # ── Context manager ─────────────────────────────────────────

    async def close(self) -> None:
        await self._client.aclose()

    async def __aenter__(self) -> AsyncHippoClient:
        return self

    async def __aexit__(self, *args: Any) -> None:
        await self.close()

    # ── Internal helpers ────────────────────────────────────────

    async def _post(self, path: str, body: dict[str, Any]) -> dict[str, Any]:
        resp = await self._client.post(path, json=body)
        _raise_for_status(resp)
        return resp.json()

    async def _get(self, path: str) -> dict[str, Any]:
        resp = await self._client.get(path)
        _raise_for_status(resp)
        return resp.json()

    async def _delete(self, path: str) -> None:
        resp = await self._client.delete(path)
        _raise_for_status(resp)

    # ── Core endpoints ──────────────────────────────────────────

    async def remember(
        self,
        statement: str,
        *,
        source_agent: str | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
    ) -> RememberResponse:
        body: dict[str, Any] = {"statement": statement}
        if source_agent is not None:
            body["source_agent"] = source_agent
        if graph is not None:
            body["graph"] = graph
        if ttl_secs is not None:
            body["ttl_secs"] = ttl_secs
        return RememberResponse.model_validate(await self._post("/remember", body))

    async def remember_batch(
        self,
        statements: list[str],
        *,
        source_agent: str | None = None,
        parallel: bool | None = None,
        graph: str | None = None,
        ttl_secs: int | None = None,
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
        return RememberBatchResponse.model_validate(await self._post("/remember/batch", body))

    async def context(
        self,
        query: str,
        *,
        limit: int | None = None,
        max_hops: int | None = None,
        graph: str | None = None,
    ) -> ContextResponse:
        body: dict[str, Any] = {"query": query}
        if limit is not None:
            body["limit"] = limit
        if max_hops is not None:
            body["max_hops"] = max_hops
        if graph is not None:
            body["graph"] = graph
        return ContextResponse.model_validate(await self._post("/context", body))

    async def ask(
        self,
        question: str,
        *,
        limit: int | None = None,
        graph: str | None = None,
        verbose: bool | None = None,
    ) -> AskResponse:
        body: dict[str, Any] = {"question": question}
        if limit is not None:
            body["limit"] = limit
        if graph is not None:
            body["graph"] = graph
        if verbose is not None:
            body["verbose"] = verbose
        return AskResponse.model_validate(await self._post("/ask", body))

    # ── Admin endpoints ─────────────────────────────────────────

    async def create_user(
        self,
        user_id: str,
        display_name: str,
        *,
        role: str | None = None,
        graphs: list[str] | None = None,
    ) -> CreateUserResponse:
        body: dict[str, Any] = {"user_id": user_id, "display_name": display_name}
        if role is not None:
            body["role"] = role
        if graphs is not None:
            body["graphs"] = graphs
        return CreateUserResponse.model_validate(await self._post("/admin/users", body))

    async def list_users(self) -> ListUsersResponse:
        return ListUsersResponse.model_validate(await self._get("/admin/users"))

    async def delete_user(self, user_id: str) -> None:
        await self._delete(f"/admin/users/{user_id}")

    async def create_key(self, user_id: str, label: str) -> CreateKeyResponse:
        return CreateKeyResponse.model_validate(
            await self._post(f"/admin/users/{user_id}/keys", {"label": label})
        )

    async def list_keys(self, user_id: str) -> ListKeysResponse:
        return ListKeysResponse.model_validate(
            await self._get(f"/admin/users/{user_id}/keys")
        )

    async def delete_key(self, user_id: str, label: str) -> None:
        await self._delete(f"/admin/users/{user_id}/keys/{label}")

    # ── Observability ───────────────────────────────────────────

    async def health(self) -> HealthResponse:
        return HealthResponse.model_validate(await self._get("/health"))
