using System.Text.Json.Serialization;

namespace Hippo.Sdk;

// ── Core requests ──

public sealed record RememberRequest
{
    [JsonPropertyName("statement")]
    public required string Statement { get; init; }

    [JsonPropertyName("source_agent")]
    public string? SourceAgent { get; init; }

    [JsonPropertyName("graph")]
    public string? Graph { get; init; }

    [JsonPropertyName("ttl_secs")]
    public int? TtlSecs { get; init; }
}

public sealed record RememberBatchRequest
{
    [JsonPropertyName("statements")]
    public required string[] Statements { get; init; }

    [JsonPropertyName("source_agent")]
    public string? SourceAgent { get; init; }

    [JsonPropertyName("parallel")]
    public bool? Parallel { get; init; }

    [JsonPropertyName("graph")]
    public string? Graph { get; init; }

    [JsonPropertyName("ttl_secs")]
    public int? TtlSecs { get; init; }
}

public sealed record ContextRequest
{
    [JsonPropertyName("query")]
    public required string Query { get; init; }

    [JsonPropertyName("limit")]
    public int? Limit { get; init; }

    [JsonPropertyName("max_hops")]
    public int? MaxHops { get; init; }

    [JsonPropertyName("graph")]
    public string? Graph { get; init; }
}

public sealed record AskRequest
{
    [JsonPropertyName("question")]
    public required string Question { get; init; }

    [JsonPropertyName("limit")]
    public int? Limit { get; init; }

    [JsonPropertyName("graph")]
    public string? Graph { get; init; }

    [JsonPropertyName("verbose")]
    public bool? Verbose { get; init; }
}

// ── Core responses ──

public sealed record RememberResponse
{
    [JsonPropertyName("entities_created")]
    public int EntitiesCreated { get; init; }

    [JsonPropertyName("entities_resolved")]
    public int EntitiesResolved { get; init; }

    [JsonPropertyName("facts_written")]
    public int FactsWritten { get; init; }

    [JsonPropertyName("contradictions_invalidated")]
    public int ContradictionsInvalidated { get; init; }
}

public sealed record RememberBatchResponse
{
    [JsonPropertyName("total")]
    public int Total { get; init; }

    [JsonPropertyName("succeeded")]
    public int Succeeded { get; init; }

    [JsonPropertyName("failed")]
    public int Failed { get; init; }

    [JsonPropertyName("results")]
    public RememberResponse[]? Results { get; init; }
}

public sealed record ContextResponse
{
    [JsonPropertyName("nodes")]
    public ContextNode[]? Nodes { get; init; }

    [JsonPropertyName("edges")]
    public ContextEdge[]? Edges { get; init; }
}

public sealed record ContextNode
{
    [JsonPropertyName("name")]
    public string? Name { get; init; }

    [JsonPropertyName("entity_type")]
    public string? EntityType { get; init; }
}

public sealed record ContextEdge
{
    [JsonPropertyName("source")]
    public string? Source { get; init; }

    [JsonPropertyName("target")]
    public string? Target { get; init; }

    [JsonPropertyName("relation")]
    public string? Relation { get; init; }
}

public sealed record AskResponse
{
    [JsonPropertyName("answer")]
    public required string Answer { get; init; }

    [JsonPropertyName("facts")]
    public string[]? Facts { get; init; }
}

// ── Admin requests ──

public sealed record CreateUserRequest
{
    [JsonPropertyName("user_id")]
    public required string UserId { get; init; }

    [JsonPropertyName("display_name")]
    public required string DisplayName { get; init; }

    [JsonPropertyName("role")]
    public string? Role { get; init; }

    [JsonPropertyName("graphs")]
    public string[]? Graphs { get; init; }
}

public sealed record CreateKeyRequest
{
    [JsonPropertyName("label")]
    public required string Label { get; init; }
}

// ── Admin responses ──

public sealed record CreateUserResponse
{
    [JsonPropertyName("user_id")]
    public required string UserId { get; init; }

    [JsonPropertyName("api_key")]
    public required string ApiKey { get; init; }
}

public sealed record ListUsersResponse
{
    [JsonPropertyName("users")]
    public UserInfo[]? Users { get; init; }
}

public sealed record UserInfo
{
    [JsonPropertyName("user_id")]
    public string? UserId { get; init; }

    [JsonPropertyName("display_name")]
    public string? DisplayName { get; init; }

    [JsonPropertyName("role")]
    public string? Role { get; init; }

    [JsonPropertyName("graphs")]
    public string[]? Graphs { get; init; }

    [JsonPropertyName("key_count")]
    public int KeyCount { get; init; }
}

public sealed record CreateKeyResponse
{
    [JsonPropertyName("user_id")]
    public required string UserId { get; init; }

    [JsonPropertyName("label")]
    public required string Label { get; init; }

    [JsonPropertyName("api_key")]
    public required string ApiKey { get; init; }
}

public sealed record ListKeysResponse
{
    [JsonPropertyName("keys")]
    public KeyInfo[]? Keys { get; init; }
}

public sealed record KeyInfo
{
    [JsonPropertyName("label")]
    public string? Label { get; init; }

    [JsonPropertyName("created_at")]
    public string? CreatedAt { get; init; }
}

// ── Observability ──

public sealed record HealthResponse
{
    [JsonPropertyName("status")]
    public required string Status { get; init; }

    [JsonPropertyName("graph")]
    public string? Graph { get; init; }
}
