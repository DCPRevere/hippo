from __future__ import annotations

from unittest.mock import MagicMock, patch

import httpx
import pytest
import respx

from hippo import (
    AskResponse,
    AsyncHippoClient,
    AuditResponse,
    AuthenticationError,
    ContextFact,
    ContextResponse,
    CorrectResponse,
    CreateKeyResponse,
    CreateUserResponse,
    ForbiddenError,
    GraphEvent,
    GraphsListResponse,
    HealthResponse,
    HippoClient,
    HippoError,
    ListKeysResponse,
    ListUsersResponse,
    RateLimitError,
    RememberBatchResponse,
    RememberResponse,
    RetractResponse,
)

BASE = "https://hippo.test"
KEY = "test-key-123"


# Fixtures ──────────────────────────────────────────────────────


@pytest.fixture()
def client() -> HippoClient:
    return HippoClient(base_url=BASE, api_key=KEY, max_retries=0)


@pytest.fixture()
def async_client() -> AsyncHippoClient:
    return AsyncHippoClient(base_url=BASE, api_key=KEY, max_retries=0)


def _fact(**overrides):
    """Sample ContextFact JSON payload, with overrides."""
    base = {
        "fact": "Alice works at Acme",
        "subject": "Alice",
        "relation_type": "WORKS_AT",
        "object": "Acme",
        "confidence": 0.95,
        "salience": 1,
        "valid_at": "2025-01-01T00:00:00Z",
        "edge_id": 42,
        "hops": 0,
        "source_agents": ["test"],
        "memory_tier": "long_term",
    }
    base.update(overrides)
    return base


# Sync tests ────────────────────────────────────────────────────


class TestRemember:
    @respx.mock
    def test_remember(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/remember").mock(
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
        respx.post(f"{BASE}/api/remember").mock(
            return_value=httpx.Response(200, json={
                "entities_created": 1,
                "entities_resolved": 0,
                "facts_written": 1,
                "contradictions_invalidated": 0,
            })
        )
        result = client.remember(
            "fact",
            graph="g1",
            ttl_secs=3600,
            source_credibility_hint=0.7,
        )
        assert result.facts_written == 1
        req = respx.calls.last.request
        body = req.content.decode()
        assert '"graph":"g1"' in body or '"graph": "g1"' in body
        assert "source_credibility_hint" in body

    @respx.mock
    def test_remember_batch(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/remember/batch").mock(
            return_value=httpx.Response(200, json={
                "total": 2,
                "succeeded": 2,
                "failed": 0,
                "results": [
                    {"statement": "fact 1", "ok": True, "facts_written": 1, "entities_created": 0},
                    {"statement": "fact 2", "ok": True, "facts_written": 1, "entities_created": 0},
                ],
            })
        )
        result = client.remember_batch(["fact 1", "fact 2"], parallel=True)
        assert isinstance(result, RememberBatchResponse)
        assert result.total == 2
        assert result.succeeded == 2
        assert result.results[0].ok is True


class TestContext:
    @respx.mock
    def test_context(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/context").mock(
            return_value=httpx.Response(200, json={
                "facts": [_fact(), _fact(subject="Bob", object="Alice", relation_type="KNOWS")],
            })
        )
        result = client.context("Tell me about Alice", limit=5, max_hops=2)
        assert isinstance(result, ContextResponse)
        assert len(result.facts) == 2
        assert isinstance(result.facts[0], ContextFact)
        assert result.facts[0].subject == "Alice"

    @respx.mock
    def test_context_with_advanced_fields(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/context").mock(
            return_value=httpx.Response(200, json={"facts": []})
        )
        client.context(
            "q",
            memory_tier_filter="working",
            at="2025-01-01T00:00:00Z",
            scoring={"w_relevance": 0.6, "w_confidence": 0.1, "w_recency": 0.2,
                     "w_salience": 0.1, "mmr_lambda": 0.5},
        )
        body = respx.calls.last.request.content.decode()
        assert "memory_tier_filter" in body
        assert "scoring" in body


class TestAsk:
    @respx.mock
    def test_ask(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/ask").mock(
            return_value=httpx.Response(200, json={
                "answer": "Alice likes cats.",
                "facts": [_fact()],
                "iterations": 1,
            })
        )
        result = client.ask("What does Alice like?", verbose=True)
        assert isinstance(result, AskResponse)
        assert result.answer == "Alice likes cats."
        assert result.facts is not None
        assert result.iterations == 1

    @respx.mock
    def test_ask_without_facts(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/ask").mock(
            return_value=httpx.Response(200, json={"answer": "I don't know.", "iterations": 1})
        )
        result = client.ask("Unknown question")
        assert result.facts is None

    @respx.mock
    def test_ask_with_max_iterations(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/ask").mock(
            return_value=httpx.Response(200, json={"answer": "ok", "iterations": 3})
        )
        client.ask("q", max_iterations=3)
        body = respx.calls.last.request.content.decode()
        assert "max_iterations" in body


class TestRetractCorrect:
    @respx.mock
    def test_retract(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/retract").mock(
            return_value=httpx.Response(200, json={"edge_id": 7, "reason": "wrong"})
        )
        result = client.retract(7, reason="wrong")
        assert isinstance(result, RetractResponse)
        assert result.edge_id == 7

    @respx.mock
    def test_correct(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/correct").mock(
            return_value=httpx.Response(200, json={
                "retracted_edge_id": 7,
                "reason": "user fix",
                "remember": {
                    "entities_created": 0,
                    "entities_resolved": 1,
                    "facts_written": 1,
                    "contradictions_invalidated": 0,
                },
            })
        )
        result = client.correct(7, "Alice is a dentist", reason="user fix")
        assert isinstance(result, CorrectResponse)
        assert result.retracted_edge_id == 7
        assert result.remember.facts_written == 1


class TestRestResources:
    @respx.mock
    def test_get_entity(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/entities/alice").mock(
            return_value=httpx.Response(200, json={"id": "alice", "name": "Alice"})
        )
        result = client.get_entity("alice")
        assert result["id"] == "alice"

    @respx.mock
    def test_delete_entity(self, client: HippoClient) -> None:
        respx.delete(f"{BASE}/api/entities/alice").mock(
            return_value=httpx.Response(200, json={"id": "alice", "name": "Alice", "edges_invalidated": 3})
        )
        result = client.delete_entity("alice")
        assert result["edges_invalidated"] == 3

    @respx.mock
    def test_entity_edges(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/entities/alice/edges").mock(
            return_value=httpx.Response(200, json=[{"edge_id": 1}])
        )
        edges = client.entity_edges("alice")
        assert len(edges) == 1

    @respx.mock
    def test_get_edge(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/edges/42").mock(
            return_value=httpx.Response(200, json={"edge_id": 42})
        )
        edge = client.get_edge(42)
        assert edge["edge_id"] == 42

    @respx.mock
    def test_edge_provenance(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/edges/42/provenance").mock(
            return_value=httpx.Response(200, json={"edge_id": 42, "supersedes": []})
        )
        prov = client.edge_provenance(42)
        assert prov["edge_id"] == 42


class TestGraphOps:
    @respx.mock
    def test_maintain(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/maintain").mock(
            return_value=httpx.Response(200, json={"ok": True})
        )
        result = client.maintain()
        assert result["ok"] is True

    @respx.mock
    def test_graph_json(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/graph").mock(
            return_value=httpx.Response(200, json={"graph": "default", "entities": [], "edges": {}})
        )
        result = client.graph()
        assert result["graph"] == "default"

    @respx.mock
    def test_list_graphs(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/graphs").mock(
            return_value=httpx.Response(200, json={"default": "default", "graphs": ["default", "other"]})
        )
        result = client.list_graphs()
        assert isinstance(result, GraphsListResponse)
        assert "other" in result.graphs

    @respx.mock
    def test_drop_graph(self, client: HippoClient) -> None:
        respx.delete(f"{BASE}/api/graphs/drop/other").mock(
            return_value=httpx.Response(200, json={"ok": True})
        )
        client.drop_graph("other")


class TestAdmin:
    @respx.mock
    def test_create_user(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/admin/users").mock(
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
        respx.get(f"{BASE}/api/admin/users").mock(
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
        respx.delete(f"{BASE}/api/admin/users/alice").mock(
            return_value=httpx.Response(204)
        )
        client.delete_user("alice")  # should not raise

    @respx.mock
    def test_create_key(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/admin/users/alice/keys").mock(
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
        respx.get(f"{BASE}/api/admin/users/alice/keys").mock(
            return_value=httpx.Response(200, json={
                "keys": [{"label": "laptop", "created_at": "2025-01-01T00:00:00Z"}],
            })
        )
        result = client.list_keys("alice")
        assert isinstance(result, ListKeysResponse)
        assert len(result.keys) == 1

    @respx.mock
    def test_delete_key(self, client: HippoClient) -> None:
        respx.delete(f"{BASE}/api/admin/users/alice/keys/laptop").mock(
            return_value=httpx.Response(204)
        )
        client.delete_key("alice", "laptop")

    @respx.mock
    def test_audit(self, client: HippoClient) -> None:
        respx.get(f"{BASE}/api/admin/audit?user_id=alice&limit=10").mock(
            return_value=httpx.Response(200, json={
                "entries": [{
                    "id": "1",
                    "user_id": "alice",
                    "action": "remember",
                    "details": "x",
                    "timestamp": "2025-01-01T00:00:00Z",
                }],
            })
        )
        result = client.audit(user_id="alice", limit=10)
        assert isinstance(result, AuditResponse)
        assert len(result.entries) == 1


class TestHealth:
    @respx.mock
    def test_health(self, client: HippoClient) -> None:
        # /health is the only path served at the root, not under /api.
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
        respx.post(f"{BASE}/api/remember").mock(
            return_value=httpx.Response(401, text="Unauthorized")
        )
        with pytest.raises(AuthenticationError) as exc_info:
            client.remember("test")
        assert exc_info.value.status_code == 401

    @respx.mock
    def test_403_raises_forbidden_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/admin/users").mock(
            return_value=httpx.Response(403, text="Forbidden")
        )
        with pytest.raises(ForbiddenError) as exc_info:
            client.create_user("x", "X")
        assert exc_info.value.status_code == 403

    @respx.mock
    def test_429_raises_rate_limit_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/remember").mock(
            return_value=httpx.Response(429, text="Too Many Requests")
        )
        with pytest.raises(RateLimitError) as exc_info:
            client.remember("test")
        assert exc_info.value.status_code == 429

    @respx.mock
    def test_500_raises_hippo_error(self, client: HippoClient) -> None:
        respx.post(f"{BASE}/api/remember").mock(
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


# Async tests ───────────────────────────────────────────────────


class TestAsyncRemember:
    @respx.mock
    @pytest.mark.asyncio
    async def test_remember(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/api/remember").mock(
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
        respx.post(f"{BASE}/api/remember/batch").mock(
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
        respx.post(f"{BASE}/api/context").mock(
            return_value=httpx.Response(200, json={"facts": []})
        )
        result = await async_client.context("query")
        assert isinstance(result, ContextResponse)


class TestAsyncAsk:
    @respx.mock
    @pytest.mark.asyncio
    async def test_ask(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/api/ask").mock(
            return_value=httpx.Response(200, json={"answer": "42", "iterations": 1})
        )
        result = await async_client.ask("meaning of life")
        assert result.answer == "42"


class TestAsyncAdmin:
    @respx.mock
    @pytest.mark.asyncio
    async def test_create_user(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/api/admin/users").mock(
            return_value=httpx.Response(200, json={"user_id": "bob", "api_key": "k"})
        )
        result = await async_client.create_user("bob", "Bob")
        assert result.user_id == "bob"

    @respx.mock
    @pytest.mark.asyncio
    async def test_list_users(self, async_client: AsyncHippoClient) -> None:
        respx.get(f"{BASE}/api/admin/users").mock(
            return_value=httpx.Response(200, json={"users": []})
        )
        result = await async_client.list_users()
        assert result.users == []

    @respx.mock
    @pytest.mark.asyncio
    async def test_delete_user(self, async_client: AsyncHippoClient) -> None:
        respx.delete(f"{BASE}/api/admin/users/bob").mock(
            return_value=httpx.Response(204)
        )
        await async_client.delete_user("bob")

    @respx.mock
    @pytest.mark.asyncio
    async def test_create_key(self, async_client: AsyncHippoClient) -> None:
        respx.post(f"{BASE}/api/admin/users/bob/keys").mock(
            return_value=httpx.Response(200, json={
                "user_id": "bob", "label": "phone", "api_key": "k2",
            })
        )
        result = await async_client.create_key("bob", "phone")
        assert result.label == "phone"

    @respx.mock
    @pytest.mark.asyncio
    async def test_list_keys(self, async_client: AsyncHippoClient) -> None:
        respx.get(f"{BASE}/api/admin/users/bob/keys").mock(
            return_value=httpx.Response(200, json={"keys": []})
        )
        result = await async_client.list_keys("bob")
        assert result.keys == []

    @respx.mock
    @pytest.mark.asyncio
    async def test_delete_key(self, async_client: AsyncHippoClient) -> None:
        respx.delete(f"{BASE}/api/admin/users/bob/keys/phone").mock(
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
        respx.post(f"{BASE}/api/remember").mock(
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


# Retry tests ───────────────────────────────────────────────────


class TestRetry:
    @respx.mock
    def test_retry_on_502_then_success(self) -> None:
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
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=0)
        route = respx.get(f"{BASE}/health")
        route.mock(return_value=httpx.Response(502, text="Bad Gateway"))
        with pytest.raises(HippoError) as exc_info:
            c.health()
        assert exc_info.value.status_code == 502
        assert route.call_count == 1

    @respx.mock
    def test_retry_respects_retry_after_header(self) -> None:
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
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=0)
        respx.post(f"{BASE}/api/remember").mock(
            return_value=httpx.Response(429, text="Too Many Requests", headers={"Retry-After": "5"})
        )
        with pytest.raises(RateLimitError) as exc_info:
            c.remember("test")
        assert exc_info.value.retry_after == 5.0

    @respx.mock
    def test_no_retry_on_4xx(self) -> None:
        c = HippoClient(base_url=BASE, api_key=KEY, max_retries=3)
        route = respx.post(f"{BASE}/api/remember")
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


# Per-request timeout tests ────────────────────────────────────


class TestPerRequestTimeout:
    @respx.mock
    def test_per_request_timeout_used(self) -> None:
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
            c.health(timeout=5.0)
        assert captured_timeouts[0] == 5.0

    @respx.mock
    def test_default_timeout_when_none(self) -> None:
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


# Response helper tests ────────────────────────────────────────


class TestResponseHelpers:
    def test_context_response_facts_about(self) -> None:
        resp = ContextResponse(facts=[
            ContextFact(**_fact(subject="Alice", object="Acme", relation_type="WORKS_AT", edge_id=1)),
            ContextFact(**_fact(subject="Bob", object="Alice", relation_type="KNOWS", edge_id=2)),
            ContextFact(**_fact(subject="Carol", object="Dave", relation_type="KNOWS", edge_id=3)),
        ])
        about = resp.facts_about("alice")
        assert {f.edge_id for f in about} == {1, 2}

    def test_context_response_find_subject(self) -> None:
        resp = ContextResponse(facts=[
            ContextFact(**_fact(subject="Alice", edge_id=1)),
            ContextFact(**_fact(subject="Bob", edge_id=2)),
        ])
        results = resp.find_subject("ALICE")
        assert len(results) == 1
        assert results[0].edge_id == 1

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
        ok_item = BatchResultItem(statement="ok", ok=True)
        err_item = BatchResultItem(statement="bad", ok=False, error="oops")
        resp = RememberBatchResponse(
            total=2, succeeded=1, failed=1,
            results=[ok_item, err_item],
        )
        failures = resp.failures
        assert len(failures) == 1


# Custom HTTP client injection ─────────────────────────────────


class TestCustomHttpClient:
    @respx.mock
    def test_sync_custom_client(self) -> None:
        custom = httpx.Client(base_url=BASE, headers={"Authorization": f"Bearer {KEY}"})
        c = HippoClient(base_url=BASE, api_key=KEY, http_client=custom)
        assert c._client is custom
        assert c._owns_client is False
        c.close()
        assert not custom.is_closed
        custom.close()

    def test_sync_default_client_owned(self) -> None:
        c = HippoClient(base_url=BASE, api_key=KEY)
        assert c._owns_client is True
        c.close()
        assert c._client.is_closed

    @pytest.mark.asyncio
    async def test_async_custom_client(self) -> None:
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


# SSE streaming tests ──────────────────────────────────────────


class TestEvents:
    def test_sync_events_streaming(self) -> None:
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
