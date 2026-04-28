from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field


# ── Shared / scoring ────────────────────────────────────────────


class ScoringParams(BaseModel, extra="allow"):
    w_relevance: float = 0.50
    w_confidence: float = 0.10
    w_recency: float = 0.25
    w_salience: float = 0.15
    mmr_lambda: float = 0.70


class LlmUsage(BaseModel, extra="allow"):
    llm_calls: int = 0
    embed_calls: int = 0
    input_tokens: int = 0
    output_tokens: int = 0


class OpExecutionTrace(BaseModel, extra="allow"):
    op: str
    outcome: str
    details: str | None = None


class RememberTrace(BaseModel, extra="allow"):
    operations: list[dict[str, Any]] = Field(default_factory=list)
    revised_operations: list[dict[str, Any]] | None = None
    execution: list[OpExecutionTrace] = Field(default_factory=list)


# ── Request models ──────────────────────────────────────────────


class RememberRequest(BaseModel):
    statement: str
    source_agent: str | None = None
    source_credibility_hint: float | None = None
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
    memory_tier_filter: str | None = None
    graph: str | None = None
    at: str | None = None
    scoring: ScoringParams | None = None


class AskRequest(BaseModel):
    question: str
    limit: int | None = None
    graph: str | None = None
    verbose: bool | None = None
    max_iterations: int | None = None


class RetractRequest(BaseModel):
    edge_id: int
    reason: str | None = None
    graph: str | None = None


class CorrectRequest(BaseModel):
    edge_id: int
    statement: str
    reason: str | None = None
    source_agent: str | None = None
    graph: str | None = None


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
    usage: LlmUsage | None = None
    trace: RememberTrace | None = None

    @property
    def was_duplicate(self) -> bool:
        """True if no new facts were written (likely a duplicate)."""
        return self.facts_written == 0


class BatchResultItem(BaseModel, extra="allow"):
    statement: str | None = None
    ok: bool | None = None
    facts_written: int | None = None
    entities_created: int | None = None
    error: str | None = None


class RememberBatchResponse(BaseModel, extra="allow"):
    total: int
    succeeded: int
    failed: int
    results: list[BatchResultItem]

    @property
    def failures(self) -> list[BatchResultItem]:
        """Return the list of failed results."""
        return [r for r in self.results if r.error is not None]


class ContextFact(BaseModel, extra="allow"):
    fact: str
    subject: str
    relation_type: str
    object: str
    confidence: float
    salience: int
    valid_at: str
    edge_id: int
    hops: int
    source_agents: list[str]
    memory_tier: str


class ContextResponse(BaseModel, extra="allow"):
    facts: list[ContextFact]

    def find_subject(self, name: str) -> list[ContextFact]:
        """Return facts whose subject matches `name` (case-insensitive)."""
        lower = name.lower()
        return [f for f in self.facts if f.subject.lower() == lower]

    def facts_about(self, entity_name: str) -> list[ContextFact]:
        """Return facts where `entity_name` is the subject or object
        (case-insensitive)."""
        lower = entity_name.lower()
        return [
            f
            for f in self.facts
            if f.subject.lower() == lower or f.object.lower() == lower
        ]


class AskResponse(BaseModel, extra="allow"):
    answer: str
    facts: list[ContextFact] | None = None
    iterations: int = 1


class RetractResponse(BaseModel, extra="allow"):
    edge_id: int
    reason: str | None = None


class CorrectResponse(BaseModel, extra="allow"):
    retracted_edge_id: int
    reason: str | None = None
    remember: RememberResponse


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


class GraphsListResponse(BaseModel, extra="allow"):
    default: str
    graphs: list[str]


class AuditEntry(BaseModel, extra="allow"):
    id: str
    user_id: str
    action: str
    details: str
    timestamp: str


class AuditResponse(BaseModel, extra="allow"):
    entries: list[AuditEntry]


# ── SSE event model ─────────────────────────────────────────────


class GraphEvent(BaseModel, extra="allow"):
    """A Server-Sent Event from the /events endpoint."""
    event: str
    data: dict
