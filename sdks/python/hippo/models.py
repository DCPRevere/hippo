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


class BatchResultItem(BaseModel, extra="allow"):
    pass


class RememberBatchResponse(BaseModel, extra="allow"):
    total: int
    succeeded: int
    failed: int
    results: list[BatchResultItem]


class ContextResponse(BaseModel, extra="allow"):
    nodes: list[dict]
    edges: list[dict]


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
