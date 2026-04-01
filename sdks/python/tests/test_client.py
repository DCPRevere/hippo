from __future__ import annotations

from unittest.mock import MagicMock, patch

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
    GraphEvent,
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
    return HippoClient(base_url=BASE, api_key=KEY, max_retries=0)


@pytest.fixture()
def async_client() -> AsyncHippoClient:
    return AsyncHippoClient(base_url=BASE, api_key=KEY, max_retries=0)


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
        c = HippoClient(base_url=BASE, max_retries=0)
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


# ── Retry tests ─────────────────────────────────────────────────


class TestRetry:
    @respx.mock
    def test_retry_on_502_then_success(self) -> None:
        """Retry on 502, then succeed on second attempt."""
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=2)
        route = respx.get(f"{BASE}/health")
        route.side_effect = [
            httpx.Response(502, text="Bad Gateway"),
            httpx.Response(200, json={"status": "ok", "graph": "default"}),
        ]
        with patch("hippo.client.time.sleep"):
            result = c.health()
        assert result.status == "ok"
        assert route.call_count == 2

    @respx.mock
    def test_retry_exhausted_raises(self) -> None:
        """After max_retries exhausted, the error status propagates."""
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=1)
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(503, text="Service Unavailable")
        )
        with patch("hippo.client.time.sleep"):
            with pytest.raises(HippoError) as exc_info:
                c.health()
        assert exc_info.value.status_code == 503

    @respx.mock
    def test_max_retries_zero_disables_retry(self) -> None:
        """max_retries=0 means no retries at all."""
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=0)
        route = respx.get(f"{BASE}/health")
        route.mock(return_value=httpx.Response(502, text="Bad Gateway"))
        with pytest.raises(HippoError) as exc_info:
            c.health()
        assert exc_info.value.status_code == 502
        assert route.call_count == 1

    @respx.mock
    def test_retry_respects_retry_after_header(self) -> None:
        """When Retry-After header is present, use its value as delay."""
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=1)
        route = respx.get(f"{BASE}/health")
        route.side_effect = [
            httpx.Response(429, text="Too Many Requests", headers={"Retry-After": "2"}),
            httpx.Response(200, json={"status": "ok", "graph": "default"}),
        ]
        with patch("hippo.client.time.sleep") as mock_sleep:
            result = c.health()
        assert result.status == "ok"
        mock_sleep.assert_called_once_with(2.0)

    @respx.mock
    def test_429_stores_retry_after_on_exception(self) -> None:
        """RateLimitError stores retry_after value when retries exhausted."""
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=0)
        respx.post(f"{BASE}/remember").mock(
            return_value=httpx.Response(429, text="Too Many Requests", headers={"Retry-After": "5"})
        )
        with pytest.raises(RateLimitError) as exc_info:
            c.remember("test")
        assert exc_info.value.retry_after == 5.0

    @respx.mock
    def test_no_retry_on_4xx(self) -> None:
        """Non-retryable 4xx errors should not be retried."""
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=3)
        route = respx.post(f"{BASE}/remember")
        route.mock(return_value=httpx.Response(401, text="Unauthorized"))
        with pytest.raises(AuthenticationError):
            c.remember("test")
        assert route.call_count == 1


class TestAsyncRetry:
    @respx.mock
    @pytest.mark.asyncio
    async def test_retry_on_502_then_success(self) -> None:
        c = AsyncHippoClient(base_url=BASE, api_key=KEY, max_retries=2)
        route = respx.get(f"{BASE}/health")
        route.side_effect = [
            httpx.Response(502, text="Bad Gateway"),
            httpx.Response(200, json={"status": "ok", "graph": "default"}),
        ]
        with patch("hippo.client.asyncio.sleep"):
            result = await c.health()
        assert result.status == "ok"
        assert route.call_count == 2


# ── Per-request timeout tests ──────────────────────────────────


class TestPerRequestTimeout:
    @respx.mock
    def test_per_request_timeout_used(self) -> None:
        """Per-request timeout is passed through to the underlying request."""
        c = HippoClient(base_url=BASE, api_key=KEY, timeout=30.0, max_retries=0)
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(200, json={"status": "ok", "graph": "default"})
        )
        # We patch _client.request to capture the timeout arg
        original_request = c._client.request
        captured_timeouts: list = []

        def capturing_request(*args, **kwargs):
            captured_timeouts.append(kwargs.get("timeout"))
            return original_request(*args, **kwargs)

        with patch.object(c._client, "request", side_effect=capturing_request):
            c.health(timeout=5.0)
        assert captured_timeouts[0] == 5.0

    @respx.mock
    def test_default_timeout_when_none(self) -> None:
        """When per-request timeout is None, falls back to client default."""
        c = HippoClient(base_url=BASE, api_key=KEY, timeout=30.0, max_retries=0)
        respx.get(f"{BASE}/health").mock(
            return_value=httpx.Response(200, json={"status": "ok", "graph": "default"})
        )
        original_request = c._client.request
        captured_timeouts: list = []

        def capturing_request(*args, **kwargs):
            captured_timeouts.append(kwargs.get("timeout"))
            return original_request(*args, **kwargs)

        with patch.object(c._client, "request", side_effect=capturing_request):
            c.health()
        assert captured_timeouts[0] == 30.0


# ── Response helper methods tests ──────────────────────────────


class TestResponseHelpers:
    def test_context_response_find_node(self) -> None:
        resp = ContextResponse(
            nodes=[
                {"id": "1", "label": "Alice"},
                {"id": "2", "label": "Bob"},
            ],
            edges=[],
        )
        node = resp.find_node("Alice")
        assert node is not None
        assert node["id"] == "1"

    def test_context_response_find_node_case_insensitive(self) -> None:
        resp = ContextResponse(
            nodes=[{"id": "1", "label": "Alice"}],
            edges=[],
        )
        assert resp.find_node("alice") is not None
        assert resp.find_node("ALICE") is not None

    def test_context_response_find_node_not_found(self) -> None:
        resp = ContextResponse(nodes=[], edges=[])
        assert resp.find_node("nobody") is None

    def test_context_response_facts_about(self) -> None:
        resp = ContextResponse(
            nodes=[
                {"id": "1", "label": "Alice"},
                {"id": "2", "label": "Bob"},
            ],
            edges=[
                {"source": "1", "target": "2", "label": "knows"},
                {"source": "2", "target": "3", "label": "likes"},
            ],
        )
        facts = resp.facts_about("Alice")
        assert len(facts) == 1
        assert facts[0]["label"] == "knows"

    def test_context_response_facts_about_empty(self) -> None:
        resp = ContextResponse(
            nodes=[{"id": "1", "label": "Alice"}],
            edges=[{"source": "2", "target": "3", "label": "unrelated"}],
        )
        assert resp.facts_about("Alice") == []

    def test_remember_response_was_duplicate_true(self) -> None:
        resp = RememberResponse(
            entities_created=0,
            entities_resolved=0,
            facts_written=0,
            contradictions_invalidated=0,
        )
        assert resp.was_duplicate is True

    def test_remember_response_was_duplicate_false(self) -> None:
        resp = RememberResponse(
            entities_created=1,
            entities_resolved=0,
            facts_written=2,
            contradictions_invalidated=0,
        )
        assert resp.was_duplicate is False

    def test_batch_response_failures_empty(self) -> None:
        resp = RememberBatchResponse(
            total=2, succeeded=2, failed=0, results=[],
        )
        assert resp.failures == []

    def test_batch_response_failures_with_errors(self) -> None:
        from hippo.models import BatchResultItem
        ok_item = BatchResultItem()
        err_item = BatchResultItem(error="something went wrong")  # type: ignore[call-arg]
        resp = RememberBatchResponse(
            total=2, succeeded=1, failed=1,
            results=[ok_item, err_item],
        )
        failures = resp.failures
        assert len(failures) == 1


# ── Custom HTTP client injection tests ─────────────────────────


class TestCustomHttpClient:
    @respx.mock
    def test_sync_custom_client(self) -> None:
        """Custom httpx.Client is used and not closed on exit."""
        custom = httpx.Client(base_url=BASE, headers={"Authorization": f"Bearer {KEY}"})
        c = HippoClient(base_url=BASE, api_key=KEY, http_client=custom)
        assert c._client is custom
        assert c._owns_client is False
        # close() should not close the injected client
        c.close()
        # Client should still be usable after close
        assert not custom.is_closed
        custom.close()

    def test_sync_default_client_owned(self) -> None:
        """Default client is owned and closed."""
        c = HippoClient(base_url=BASE, api_key=KEY)
        assert c._owns_client is True
        c.close()
        assert c._client.is_closed

    @pytest.mark.asyncio
    async def test_async_custom_client(self) -> None:
        """Custom httpx.AsyncClient is used and not closed on exit."""
        custom = httpx.AsyncClient(base_url=BASE, headers={"Authorization": f"Bearer {KEY}"})
        c = AsyncHippoClient(base_url=BASE, api_key=KEY, http_client=custom)
        assert c._client is custom
        assert c._owns_client is False
        await c.close()
        assert not custom.is_closed
        await custom.aclose()

    @pytest.mark.asyncio
    async def test_async_default_client_owned(self) -> None:
        c = AsyncHippoClient(base_url=BASE, api_key=KEY)
        assert c._owns_client is True
        await c.close()
        assert c._client.is_closed


# ── SSE streaming tests ────────────────────────────────────────


class TestEvents:
    def test_sync_events_streaming(self) -> None:
        """Test sync events() parses SSE stream correctly."""
        sse_content = (
            "event: entity_created\n"
            'data: {"name": "Alice"}\n'
            "\n"
            "event: edge_created\n"
            'data: {"source": "Alice", "target": "Bob"}\n'
            "\n"
        )

        mock_response = MagicMock()
        mock_response.is_success = True
        mock_response.status_code = 200
        mock_response.iter_lines.return_value = iter(sse_content.splitlines())

        class SyncStreamCM:
            def __enter__(self_):
                return mock_response
            def __exit__(self_, *args):
                pass

        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=0)
        with patch.object(c._client, "stream", return_value=SyncStreamCM()):
            events = list(c.events(graph="mydb"))

        assert len(events) == 2
        assert isinstance(events[0], GraphEvent)
        assert events[0].event == "entity_created"
        assert events[0].data == {"name": "Alice"}
        assert events[1].event == "edge_created"
        assert events[1].data == {"source": "Alice", "target": "Bob"}

    @pytest.mark.asyncio
    async def test_async_events_streaming(self) -> None:
        """Test async events() parses SSE stream correctly."""
        sse_lines = [
            "event: entity_created",
            'data: {"name": "Alice"}',
            "",
            "event: edge_created",
            'data: {"source": "Alice", "target": "Bob"}',
            "",
        ]

        async def async_line_iter():
            for line in sse_lines:
                yield line

        mock_response = MagicMock()
        mock_response.is_success = True
        mock_response.status_code = 200
        mock_response.aiter_lines = async_line_iter

        class AsyncStreamCM:
            async def __aenter__(self):
                return mock_response
            async def __aexit__(self, *args):
                pass

        c = AsyncHippoClient(base_url=BASE, api_key=KEY, max_retries=0)
        with patch.object(c._client, "stream", return_value=AsyncStreamCM()):
            events = []
            async for event in c.events(graph="mydb"):
                events.append(event)

        assert len(events) == 2
        assert events[0].event == "entity_created"
        assert events[0].data == {"name": "Alice"}
