from __future__ import annotations

import httpx
import pytest
import respx

from hippo import (
    AskResponse,
    AsyncHippoClient,
    AuthenticationError,
    ContextResponse,
    CreateKeyResponse,
    CreateUserResponse,
    ForbiddenError,
    HealthResponse,
    HippoClient,
    HippoError,
    ListKeysResponse,
    ListUsersResponse,
    RateLimitError,
    RememberBatchResponse,
    RememberResponse,
)

BASE = "https://hippo.test"
KEY = "test-key-123"


# ── Fixtures ────────────────────────────────────────────────────


@pytest.fixture()
def client() -> HippoClient:
    return HippoClient(base_url=BASE, api_key=KEY)


@pytest.fixture()
def async_client() -> AsyncHippoClient:
    return AsyncHippoClient(base_url=BASE, api_key=KEY)


# ── Sync tests ──────────────────────────────────────────────────


class TestRemember:
    @respx.mock
    def test_remember(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(200, json={
                "entities_created": 2,
                "entities_resolved": 1,
                "facts_written": 3,
                "contradictions_invalidated": 0,
            })
        )
        result = client.remember("Alice likes cats", source_agent="test")
        assert isinstance(result, RememberResponse)
        assert result.entities_created == 2
        assert result.facts_written == 3

    @respx.mock
    def test_remember_with_optional_fields(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(200, json={
                "entities_created": 1,
                "entities_resolved": 0,
                "facts_written": 1,
                "contradictions_invalidated": 0,
            })
        )
        result = client.remember("fact", graph="g1", ttl_secs=3600)
        assert result.facts_written == 1
        req = respx.calls.last.request
        body = req.content.decode()
        assert '"graph": "g1"' in body or '"graph":"g1"' in body

    @respx.mock
    def test_remember_batch(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/remember/batch").mock(
            return_value=httpx.Response(200, json={
                "total": 2,
                "succeeded": 2,
                "failed": 0,
                "results": [],
            })
        )
        result = client.remember_batch(["fact 1", "fact 2"], parallel=True)
        assert isinstance(result, RememberBatchResponse)
        assert result.total == 2
        assert result.succeeded == 2


class TestContext:
    @respx.mock
    def test_context(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/context").mock(
            return_value=httpx.Response(200, json={
                "nodes": [{"id": "1", "label": "Alice"}],
                "edges": [{"source": "1", "target": "2", "label": "knows"}],
            })
        )
        result = client.context("Tell me about Alice", limit=5, max_hops=2)
        assert isinstance(result, ContextResponse)
        assert len(result.nodes) == 1
        assert len(result.edges) == 1


class TestAsk:
    @respx.mock
    def test_ask(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/ask").mock(
            return_value=httpx.Response(200, json={
                "answer": "Alice likes cats.",
                "facts": [{"text": "Alice likes cats"}],
            })
        )
        result = client.ask("What does Alice like?", verbose=True)
        assert isinstance(result, AskResponse)
        assert result.answer == "Alice likes cats."
        assert result.facts is not None

    @respx.mock
    def test_ask_without_facts(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/ask").mock(
            return_value=httpx.Response(200, json={"answer": "I don't know."})
        )
        result = client.ask("Unknown question")
        assert result.facts is None


class TestAdmin:
    @respx.mock
    def test_create_user(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/admin/users").mock(
            return_value=httpx.Response(200, json={
                "user_id": "alice",
                "api_key": "key-abc",
            })
        )
        result = client.create_user("alice", "Alice", role="reader", graphs=["default"])
        assert isinstance(result, CreateUserResponse)
        assert result.api_key == "key-abc"

    @respx.mock
    def test_list_users(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/admin/users").mock(
            return_value=httpx.Response(200, json={
                "users": [{
                    "user_id": "alice",
                    "display_name": "Alice",
                    "role": "reader",
                    "graphs": ["default"],
                    "key_count": 1,
                }],
            })
        )
        result = client.list_users()
        assert isinstance(result, ListUsersResponse)
        assert len(result.users) == 1
        assert result.users[0].user_id == "alice"

    @respx.mock
    def test_delete_user(self, client: HippoClient) -> None:
        respx.delete(f"{BASE}/admin/users/alice").mock(
            return_value=httpx.Response(204)
        )
        client.delete_user("alice")  # should not raise

    @respx.mock
    def test_create_key(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/admin/users/alice/keys").mock(
            return_value=httpx.Response(200, json={
                "user_id": "alice",
                "label": "laptop",
                "api_key": "key-xyz",
            })
        )
        result = client.create_key("alice", "laptop")
        assert isinstance(result, CreateKeyResponse)
        assert result.label == "laptop"

    @respx.mock
    def test_list_keys(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/admin/users/alice/keys").mock(
            return_value=httpx.Response(200, json={
                "keys": [{"label": "laptop", "created_at": "2025-01-01T00:00:00Z"}],
            })
        )
        result = client.list_keys("alice")
        assert isinstance(result, ListKeysResponse)
        assert len(result.keys) == 1

    @respx.mock
    def test_delete_key(self, client: HippoClient) -> None:
        respx.delete(f"{BASE}/admin/users/alice/keys/laptop").mock(
            return_value=httpx.Response(204)
        )
        client.delete_key("alice", "laptop")


class TestHealth:
    @respx.mock
    def test_health(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(200, json={
                "status": "ok",
                "graph": "default",
            })
        )
        result = client.health()
        assert isinstance(result, HealthResponse)
        assert result.status == "ok"


class TestErrorHandling:
    @respx.mock
    def test_401_raises_authentication_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(401, text="Unauthorized")
        )
        with pytest.raises(AuthenticationError) as exc_info:
            client.remember("test")
        assert exc_info.value.status_code == 401

    @respx.mock
    def test_403_raises_forbidden_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/admin/users").mock(
            return_value=httpx.Response(403, text="Forbidden")
        )
        with pytest.raises(ForbiddenError) as exc_info:
            client.create_user("x", "X")
        assert exc_info.value.status_code == 403

    @respx.mock
    def test_429_raises_rate_limit_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(429, text="Too Many Requests")
        )
        with pytest.raises(RateLimitError) as exc_info:
            client.remember("test")
        assert exc_info.value.status_code == 429

    @respx.mock
    def test_500_raises_hippo_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(500, text="Internal Server Error")
        )
        with pytest.raises(HippoError) as exc_info:
            client.remember("test")
        assert exc_info.value.status_code == 500
        assert exc_info.value.body == "Internal Server Error"


class TestAuth:
    @respx.mock
    def test_bearer_token_sent(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(200, json={"status": "ok", "graph": "default"})
        )
        client.health()
        req = respx.calls.last.request
        assert req.headers["Authorization"] == f"Bearer {KEY}"

    @respx.mock
    def test_no_auth_header_when_no_key(self) -> None:
        c = HippoClient(base_url=BASE)
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(200, json={"status": "ok", "graph": "default"})
        )
        c.health()
        req = respx.calls.last.request
        assert "Authorization" not in req.headers


# ── Async tests ─────────────────────────────────────────────────


class TestAsyncRemember:
    @respx.mock
    @pytest.mark.asyncio
    async def test_remember(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(200, json={
                "entities_created": 1,
                "entities_resolved": 0,
                "facts_written": 1,
                "contradictions_invalidated": 0,
            })
        )
        result = await async_client.remember("Bob likes dogs")
        assert isinstance(result, RememberResponse)
        assert result.facts_written == 1

    @respx.mock
    @pytest.mark.asyncio
    async def test_remember_batch(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/remember/batch").mock(
            return_value=httpx.Response(200, json={
                "total": 1, "succeeded": 1, "failed": 0, "results": [],
            })
        )
        result = await async_client.remember_batch(["fact"])
        assert result.succeeded == 1


class TestAsyncContext:
    @respx.mock
    @pytest.mark.asyncio
    async def test_context(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/context").mock(
            return_value=httpx.Response(200, json={"nodes": [], "edges": []})
        )
        result = await async_client.context("query")
        assert isinstance(result, ContextResponse)


class TestAsyncAsk:
    @respx.mock
    @pytest.mark.asyncio
    async def test_ask(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/ask").mock(
            return_value=httpx.Response(200, json={"answer": "42"})
        )
        result = await async_client.ask("meaning of life")
        assert result.answer == "42"


class TestAsyncAdmin:
    @respx.mock
    @pytest.mark.asyncio
    async def test_create_user(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/admin/users").mock(
            return_value=httpx.Response(200, json={"user_id": "bob", "api_key": "k"})
        )
        result = await async_client.create_user("bob", "Bob")
        assert result.user_id == "bob"

    @respx.mock
    @pytest.mark.asyncio
    async def test_list_users(self, async_client: AsyncHippoClient) -> None:
        respx.get(f"{BASE}/admin/users").mock(
            return_value=httpx.Response(200, json={"users": []})
        )
        result = await async_client.list_users()
        assert result.users == []

    @respx.mock
    @pytest.mark.asyncio
    async def test_delete_user(self, async_client: AsyncHippoClient) -> None:
        respx.delete(f"{BASE}/admin/users/bob").mock(
            return_value=httpx.Response(204)
        )
        await async_client.delete_user("bob")

    @respx.mock
    @pytest.mark.asyncio
    async def test_create_key(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/admin/users/bob/keys").mock(
            return_value=httpx.Response(200, json={
                "user_id": "bob", "label": "phone", "api_key": "k2",
            })
        )
        result = await async_client.create_key("bob", "phone")
        assert result.label == "phone"

    @respx.mock
    @pytest.mark.asyncio
    async def test_list_keys(self, async_client: AsyncHippoClient) -> None:
        respx.get(f"{BASE}/admin/users/bob/keys").mock(
            return_value=httpx.Response(200, json={"keys": []})
        )
        result = await async_client.list_keys("bob")
        assert result.keys == []

    @respx.mock
    @pytest.mark.asyncio
    async def test_delete_key(self, async_client: AsyncHippoClient) -> None:
        respx.delete(f"{BASE}/admin/users/bob/keys/phone").mock(
            return_value=httpx.Response(204)
        )
        await async_client.delete_key("bob", "phone")


class TestAsyncHealth:
    @respx.mock
    @pytest.mark.asyncio
    async def test_health(self, async_client: AsyncHippoClient) -> None:
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(200, json={"status": "ok", "graph": "default"})
        )
        result = await async_client.health()
        assert result.status == "ok"


class TestAsyncErrors:
    @respx.mock
    @pytest.mark.asyncio
    async def test_401(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(401, text="Unauthorized")
        )
        with pytest.raises(AuthenticationError):
            await async_client.remember("test")


class TestContextManager:
    def test_sync_context_manager(self) -> None:
        with HippoClient(base_url=BASE, api_key=KEY) as c:
            assert c.base_url == BASE

    @pytest.mark.asyncio
    async def test_async_context_manager(self) -> None:
        async with AsyncHippoClient(base_url=BASE, api_key=KEY) as c:
            assert c.base_url == BASE
