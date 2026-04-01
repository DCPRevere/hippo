from __future__ import annotations

from pydantic import BaseModel


# ── Request models ──────────────────────────────────────────────


class RememberRequest(BaseModel):
    statement: str
    source_agent: str | None = None
    graph: str | None = None
    ttl_secs: int | None = None


class RememberBatchRequest(BaseModel):
    statements: list[str]
    source_agent: str | None = None
    parallel: bool | None = None
    graph: str | None = None
    ttl_secs: int | None = None


class ContextRequest(BaseModel):
    query: str
    limit: int | None = None
    max_hops: int | None = None
    graph: str | None = None


class AskRequest(BaseModel):
    question: str
    limit: int | None = None
    graph: str | None = None
    verbose: bool | None = None


class CreateUserRequest(BaseModel):
    user_id: str
    display_name: str
    role: str | None = None
    graphs: list[str] | None = None


class CreateKeyRequest(BaseModel):
    label: str


# ── Response models ─────────────────────────────────────────────


class RememberResponse(BaseModel, extra="allow"):
    entities_created: int
    entities_resolved: int
    facts_written: int
    contradictions_invalidated: int

    @property
    def was_duplicate(self) -> bool:
        """True if no new facts were written (likely a duplicate)."""
        return self.facts_written == 0


class BatchResultItem(BaseModel, extra="allow"):
    pass


class RememberBatchResponse(BaseModel, extra="allow"):
    total: int
    succeeded: int
    failed: int
    results: list[BatchResultItem]

    @property
    def failures(self) -> list[BatchResultItem]:
        """Return the list of failed results."""
        return [r for r in self.results if getattr(r, "error", None) is not None]


class ContextResponse(BaseModel, extra="allow"):
    nodes: list[dict]
    edges: list[dict]

    def find_node(self, name: str) -> dict | None:
        """Find a node by name/label (case-insensitive)."""
        lower = name.lower()
        for node in self.nodes:
            if node.get("label", "").lower() == lower:
                return node
            if node.get("name", "").lower() == lower:
                return node
        return None

    def facts_about(self, entity_name: str) -> list[dict]:
        """Filter edges involving an entity by name (case-insensitive)."""
        lower = entity_name.lower()
        # Build a set of node IDs matching the entity name
        node_ids: set[str] = set()
        for node in self.nodes:
            if node.get("label", "").lower() == lower or node.get("name", "").lower() == lower:
                node_ids.add(str(node.get("id", "")))
        return [
            edge for edge in self.edges
            if str(edge.get("source", "")) in node_ids or str(edge.get("target", "")) in node_ids
        ]


class AskResponse(BaseModel, extra="allow"):
    answer: str
    facts: list[dict] | None = None


class CreateUserResponse(BaseModel):
    user_id: str
    api_key: str


class UserInfo(BaseModel, extra="allow"):
    user_id: str
    display_name: str
    role: str
    graphs: list[str]
    key_count: int


class ListUsersResponse(BaseModel):
    users: list[UserInfo]


class CreateKeyResponse(BaseModel):
    user_id: str
    label: str
    api_key: str


class KeyInfo(BaseModel, extra="allow"):
    label: str
    created_at: str


class ListKeysResponse(BaseModel):
    keys: list[KeyInfo]


class HealthResponse(BaseModel, extra="allow"):
    status: str
    graph: str


# ── SSE event model ─────────────────────────────────────────────


class GraphEvent(BaseModel, extra="allow"):
    """A Server-Sent Event from the /events endpoint."""
    event: str
    data: dict
