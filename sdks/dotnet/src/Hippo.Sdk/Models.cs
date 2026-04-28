using System.Text.Json.Serialization;

namespace Hippo.Sdk;

// ── Shared / scoring ──

public sealed record ScoringParams
{
    [JsonPropertyName("w_relevance")] public required float WRelevance { get; init; }
    [JsonPropertyName("w_confidence")] public required float WConfidence { get; init; }
    [JsonPropertyName("w_recency")] public required float WRecency { get; init; }
    [JsonPropertyName("w_salience")] public required float WSalience { get; init; }
    [JsonPropertyName("mmr_lambda")] public required float MmrLambda { get; init; }
}

public sealed record LlmUsage
{
    [JsonPropertyName("llm_calls")] public int LlmCalls { get; init; }
    [JsonPropertyName("embed_calls")] public int EmbedCalls { get; init; }
    [JsonPropertyName("input_tokens")] public int InputTokens { get; init; }
    [JsonPropertyName("output_tokens")] public int OutputTokens { get; init; }
}

public sealed record OpExecutionTrace
{
    [JsonPropertyName("op")] public required string Op { get; init; }
    [JsonPropertyName("outcome")] public required string Outcome { get; init; }
    [JsonPropertyName("details")] public string? Details { get; init; }
}

public sealed record RememberTrace
{
    [JsonPropertyName("operations")]
    public Dictionary<string, object>[] Operations { get; init; } = [];

    [JsonPropertyName("revised_operations")]
    public Dictionary<string, object>[]? RevisedOperations { get; init; }

    [JsonPropertyName("execution")]
    public OpExecutionTrace[] Execution { get; init; } = [];
}

// ── Core requests ──

public sealed record RememberRequest
{
    [JsonPropertyName("statement")] public required string Statement { get; init; }
    [JsonPropertyName("source_agent")] public string? SourceAgent { get; init; }
    [JsonPropertyName("source_credibility_hint")] public float? SourceCredibilityHint { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
    [JsonPropertyName("ttl_secs")] public int? TtlSecs { get; init; }
}

public sealed record RememberBatchRequest
{
    [JsonPropertyName("statements")] public required string[] Statements { get; init; }
    [JsonPropertyName("source_agent")] public string? SourceAgent { get; init; }
    [JsonPropertyName("parallel")] public bool? Parallel { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
    [JsonPropertyName("ttl_secs")] public int? TtlSecs { get; init; }
}

public sealed record ContextRequest
{
    [JsonPropertyName("query")] public required string Query { get; init; }
    [JsonPropertyName("limit")] public int? Limit { get; init; }
    [JsonPropertyName("max_hops")] public int? MaxHops { get; init; }
    [JsonPropertyName("memory_tier_filter")] public string? MemoryTierFilter { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
    /// <summary>ISO-8601 timestamp; the server filters edges valid at this instant.</summary>
    [JsonPropertyName("at")] public string? At { get; init; }
    [JsonPropertyName("scoring")] public ScoringParams? Scoring { get; init; }
}

public sealed record AskRequest
{
    [JsonPropertyName("question")] public required string Question { get; init; }
    [JsonPropertyName("limit")] public int? Limit { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
    [JsonPropertyName("verbose")] public bool? Verbose { get; init; }
    [JsonPropertyName("max_iterations")] public int? MaxIterations { get; init; }
}

public sealed record RetractRequest
{
    [JsonPropertyName("edge_id")] public required long EdgeId { get; init; }
    [JsonPropertyName("reason")] public string? Reason { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
}

public sealed record CorrectRequest
{
    [JsonPropertyName("edge_id")] public required long EdgeId { get; init; }
    [JsonPropertyName("statement")] public required string Statement { get; init; }
    [JsonPropertyName("reason")] public string? Reason { get; init; }
    [JsonPropertyName("source_agent")] public string? SourceAgent { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
}

// ── Core responses ──

public sealed record RememberResponse
{
    [JsonPropertyName("entities_created")] public int EntitiesCreated { get; init; }
    [JsonPropertyName("entities_resolved")] public int EntitiesResolved { get; init; }
    [JsonPropertyName("facts_written")] public int FactsWritten { get; init; }
    [JsonPropertyName("contradictions_invalidated")] public int ContradictionsInvalidated { get; init; }
    [JsonPropertyName("usage")] public LlmUsage? Usage { get; init; }
    [JsonPropertyName("trace")] public RememberTrace? Trace { get; init; }
}

public sealed record BatchRememberResult
{
    [JsonPropertyName("statement")] public required string Statement { get; init; }
    [JsonPropertyName("ok")] public bool Ok { get; init; }
    [JsonPropertyName("facts_written")] public int? FactsWritten { get; init; }
    [JsonPropertyName("entities_created")] public int? EntitiesCreated { get; init; }
    [JsonPropertyName("error")] public string? Error { get; init; }
}

public sealed record RememberBatchResponse
{
    [JsonPropertyName("total")] public int Total { get; init; }
    [JsonPropertyName("succeeded")] public int Succeeded { get; init; }
    [JsonPropertyName("failed")] public int Failed { get; init; }
    [JsonPropertyName("results")] public BatchRememberResult[]? Results { get; init; }
}

public sealed record ContextFact
{
    [JsonPropertyName("fact")] public required string Fact { get; init; }
    [JsonPropertyName("subject")] public required string Subject { get; init; }
    [JsonPropertyName("relation_type")] public required string RelationType { get; init; }
    [JsonPropertyName("object")] public required string Object { get; init; }
    [JsonPropertyName("confidence")] public float Confidence { get; init; }
    [JsonPropertyName("salience")] public long Salience { get; init; }
    [JsonPropertyName("valid_at")] public required string ValidAt { get; init; }
    [JsonPropertyName("edge_id")] public long EdgeId { get; init; }
    [JsonPropertyName("hops")] public int Hops { get; init; }
    [JsonPropertyName("source_agents")] public string[] SourceAgents { get; init; } = [];
    [JsonPropertyName("memory_tier")] public required string MemoryTier { get; init; }
}

public sealed record ContextResponse
{
    [JsonPropertyName("facts")] public ContextFact[] Facts { get; init; } = [];
}

public sealed record AskResponse
{
    [JsonPropertyName("answer")] public required string Answer { get; init; }
    [JsonPropertyName("facts")] public ContextFact[]? Facts { get; init; }
    [JsonPropertyName("iterations")] public int Iterations { get; init; }
}

public sealed record RetractResponse
{
    [JsonPropertyName("edge_id")] public long EdgeId { get; init; }
    [JsonPropertyName("reason")] public string? Reason { get; init; }
}

public sealed record CorrectResponse
{
    [JsonPropertyName("retracted_edge_id")] public long RetractedEdgeId { get; init; }
    [JsonPropertyName("reason")] public string? Reason { get; init; }
    [JsonPropertyName("remember")] public required RememberResponse Remember { get; init; }
}

// ── Admin requests ──

public sealed record CreateUserRequest
{
    [JsonPropertyName("user_id")] public required string UserId { get; init; }
    [JsonPropertyName("display_name")] public required string DisplayName { get; init; }
    [JsonPropertyName("role")] public string? Role { get; init; }
    [JsonPropertyName("graphs")] public string[]? Graphs { get; init; }
}

public sealed record CreateKeyRequest
{
    [JsonPropertyName("label")] public required string Label { get; init; }
}

// ── Admin responses ──

public sealed record CreateUserResponse
{
    [JsonPropertyName("user_id")] public required string UserId { get; init; }
    [JsonPropertyName("api_key")] public required string ApiKey { get; init; }
}

public sealed record ListUsersResponse
{
    [JsonPropertyName("users")] public UserInfo[]? Users { get; init; }
}

public sealed record UserInfo
{
    [JsonPropertyName("user_id")] public string? UserId { get; init; }
    [JsonPropertyName("display_name")] public string? DisplayName { get; init; }
    [JsonPropertyName("role")] public string? Role { get; init; }
    [JsonPropertyName("graphs")] public string[]? Graphs { get; init; }
    [JsonPropertyName("key_count")] public int KeyCount { get; init; }
}

public sealed record CreateKeyResponse
{
    [JsonPropertyName("user_id")] public required string UserId { get; init; }
    [JsonPropertyName("label")] public required string Label { get; init; }
    [JsonPropertyName("api_key")] public required string ApiKey { get; init; }
}

public sealed record ListKeysResponse
{
    [JsonPropertyName("keys")] public KeyInfo[]? Keys { get; init; }
}

public sealed record KeyInfo
{
    [JsonPropertyName("label")] public string? Label { get; init; }
    [JsonPropertyName("created_at")] public string? CreatedAt { get; init; }
}

public sealed record GraphsListResponse
{
    [JsonPropertyName("default")] public required string Default { get; init; }
    [JsonPropertyName("graphs")] public string[] Graphs { get; init; } = [];
}

public sealed record AuditEntry
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("user_id")] public required string UserId { get; init; }
    [JsonPropertyName("action")] public required string Action { get; init; }
    [JsonPropertyName("details")] public required string Details { get; init; }
    [JsonPropertyName("timestamp")] public required string Timestamp { get; init; }
}

public sealed record AuditResponse
{
    [JsonPropertyName("entries")] public AuditEntry[] Entries { get; init; } = [];
}

// ── SSE events ──

/// <summary>A server-sent event from the /events endpoint.</summary>
public sealed record GraphEvent
{
    /// <summary>The SSE event type (the <c>event:</c> field).</summary>
    public string? Event { get; init; }

    /// <summary>The JSON payload (the <c>data:</c> field).</summary>
    public string? Data { get; init; }
}

// ── Observability ──

public sealed record HealthResponse
{
    [JsonPropertyName("status")] public required string Status { get; init; }
    [JsonPropertyName("graph")] public string? Graph { get; init; }
}
