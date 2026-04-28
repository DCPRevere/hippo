using System.Net;
using System.Text;
using System.Text.Json;

namespace Hippo.Sdk.Tests;

internal sealed class MockHttpHandler : HttpMessageHandler
{
    private readonly Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> _handler;

    public MockHttpHandler(Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> handler)
        => _handler = handler;

    protected override Task<HttpResponseMessage> SendAsync(
        HttpRequestMessage request, CancellationToken ct)
        => _handler(request, ct);
}

public class HippoClientTests
{
    private static HttpResponseMessage JsonResponse(object body, HttpStatusCode status = HttpStatusCode.OK)
    {
        var json = JsonSerializer.Serialize(body);
        return new HttpResponseMessage(status)
        {
            Content = new StringContent(json, Encoding.UTF8, "application/json"),
        };
    }

    private static (HippoClient client, MockHttpHandler handler) CreateClient(
        Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> callback,
        string? apiKey = "test-key",
        int maxRetries = 0,
        IHippoLogger? logger = null)
    {
        var handler = new MockHttpHandler(callback);
        var http = new HttpClient(handler);
        var client = new HippoClient("http://localhost:21693", apiKey, http,
            maxRetries: maxRetries, logger: logger);
        return (client, handler);
    }

    private static object SampleFact(string subject = "Alice", string @object = "Acme") => new
    {
        fact = $"{subject} works at {@object}",
        subject,
        relation_type = "WORKS_AT",
        @object,
        confidence = 0.95,
        salience = 1,
        valid_at = "2025-01-01T00:00:00Z",
        edge_id = 42,
        hops = 0,
        source_agents = new[] { "test" },
        memory_tier = "long_term",
    };

    private static object SampleRemember() => new
    {
        entities_created = 0,
        entities_resolved = 0,
        facts_written = 0,
        contradictions_invalidated = 0,
        usage = new { llm_calls = 0, embed_calls = 0, input_tokens = 0, output_tokens = 0 },
        trace = new { operations = Array.Empty<object>(), execution = Array.Empty<object>() },
    };

    // ── Auth header ──

    [Fact]
    public async Task BearerTokenIsSentInAuthorizationHeader()
    {
        string? captured = null;
        var (client, _) = CreateClient((req, _) =>
        {
            captured = req.Headers.Authorization?.ToString();
            return Task.FromResult(JsonResponse(new { status = "ok", graph = "default" }));
        });

        await client.HealthAsync();
        Assert.Equal("Bearer test-key", captured);
    }

    [Fact]
    public async Task NoAuthHeaderWhenApiKeyIsNull()
    {
        bool hadAuth = true;
        var (client, _) = CreateClient((req, _) =>
        {
            hadAuth = req.Headers.Authorization is not null;
            return Task.FromResult(JsonResponse(new { status = "ok", graph = "default" }));
        }, apiKey: null);

        await client.HealthAsync();
        Assert.False(hadAuth);
    }

    // ── Health ──

    [Fact]
    public async Task HealthAsync_ReturnsStatus()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { status = "healthy", graph = "default" }));
        });

        var result = await client.HealthAsync();
        Assert.Equal("healthy", result.Status);
        Assert.Equal("default", result.Graph);
        // /health is the only path served at the root, no /api prefix.
        Assert.Equal("/health", path);
    }

    // ── Remember ──

    [Fact]
    public async Task RememberAsync_PostsStatementToApiPrefix()
    {
        HttpMethod? method = null;
        string? path = null;
        string? body = null;

        var (client, _) = CreateClient(async (req, _) =>
        {
            method = req.Method;
            path = req.RequestUri?.AbsolutePath;
            body = await req.Content!.ReadAsStringAsync();
            return JsonResponse(SampleRemember());
        });

        var result = await client.RememberAsync(new RememberRequest
        {
            Statement = "Alice likes Bob",
            SourceAgent = "test",
            SourceCredibilityHint = 0.7f,
            Graph = "g1",
            TtlSecs = 600,
        });

        Assert.Equal(HttpMethod.Post, method);
        Assert.Equal("/api/remember", path);
        Assert.Contains("\"statement\":\"Alice likes Bob\"", body);
        Assert.Contains("\"source_agent\":\"test\"", body);
        Assert.Contains("\"source_credibility_hint\":0.7", body);
        Assert.Contains("\"graph\":\"g1\"", body);
        Assert.Contains("\"ttl_secs\":600", body);
        Assert.NotNull(result);
    }

    [Fact]
    public async Task RememberBatchAsync_PostsStatementsAndReturnsSummary()
    {
        string? path = null;

        var (client, _) = CreateClient(async (req, ct) =>
        {
            path = req.RequestUri?.AbsolutePath;
            await req.Content!.ReadAsStringAsync(ct);
            return JsonResponse(new
            {
                total = 2,
                succeeded = 2,
                failed = 0,
                results = new[]
                {
                    new { statement = "A is B", ok = true, facts_written = 1, entities_created = 0 },
                    new { statement = "C is D", ok = true, facts_written = 1, entities_created = 0 },
                },
            });
        });

        var result = await client.RememberBatchAsync(new RememberBatchRequest
        {
            Statements = ["A is B", "C is D"],
            Parallel = true,
        });

        Assert.Equal("/api/remember/batch", path);
        Assert.Equal(2, result.Total);
        Assert.Equal(2, result.Succeeded);
        Assert.Equal(0, result.Failed);
        Assert.Equal(2, result.Results!.Length);
        Assert.True(result.Results![0].Ok);
    }

    // ── Context ──

    [Fact]
    public async Task ContextAsync_ReturnsListOfFacts()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                facts = new[] { SampleFact() },
            }));
        });

        var result = await client.ContextAsync(new ContextRequest
        {
            Query = "Alice",
            Limit = 10,
            MaxHops = 2,
        });

        Assert.Equal("/api/context", path);
        Assert.Single(result.Facts);
        Assert.Equal("Alice", result.Facts[0].Subject);
        Assert.Equal("WORKS_AT", result.Facts[0].RelationType);
    }

    [Fact]
    public async Task ContextAsync_SendsAdvancedFields()
    {
        string? body = null;
        var (client, _) = CreateClient(async (req, _) =>
        {
            body = await req.Content!.ReadAsStringAsync();
            return JsonResponse(new { facts = Array.Empty<object>() });
        });

        await client.ContextAsync(new ContextRequest
        {
            Query = "q",
            MemoryTierFilter = "working",
            At = "2025-01-01T00:00:00Z",
            Scoring = new ScoringParams
            {
                WRelevance = 0.6f, WConfidence = 0.1f, WRecency = 0.2f,
                WSalience = 0.1f, MmrLambda = 0.5f,
            },
        });

        Assert.Contains("memory_tier_filter", body);
        Assert.Contains("scoring", body);
    }

    // ── Ask ──

    [Fact]
    public async Task AskAsync_ReturnsAnswerWithIterations()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                answer = "Bob",
                facts = new[] { SampleFact() },
                iterations = 1,
            }));
        });

        var result = await client.AskAsync(new AskRequest
        {
            Question = "Who does Alice like?",
            Verbose = true,
            MaxIterations = 1,
        });

        Assert.Equal("/api/ask", path);
        Assert.Equal("Bob", result.Answer);
        Assert.Single(result.Facts!);
        Assert.Equal(1, result.Iterations);
    }

    // ── Retract / Correct ──

    [Fact]
    public async Task RetractAsync_PostsToApiRetract()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { edge_id = 7, reason = "wrong" }));
        });

        var result = await client.RetractAsync(new RetractRequest { EdgeId = 7, Reason = "wrong" });
        Assert.Equal("/api/retract", path);
        Assert.Equal(7, result.EdgeId);
    }

    [Fact]
    public async Task CorrectAsync_PostsToApiCorrect()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                retracted_edge_id = 7,
                reason = (string?)null,
                remember = SampleRemember(),
            }));
        });

        var result = await client.CorrectAsync(new CorrectRequest
        {
            EdgeId = 7,
            Statement = "Alice is a dentist",
        });
        Assert.Equal("/api/correct", path);
        Assert.Equal(7, result.RetractedEdgeId);
    }

    // ── REST resources ──

    [Fact]
    public async Task GetEntityAsync_HitsApiPath()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { id = "alice", name = "Alice" }));
        });

        var elem = await client.GetEntityAsync("alice");
        Assert.Equal("/api/entities/alice", path);
        Assert.Equal("alice", elem.GetProperty("id").GetString());
    }

    [Fact]
    public async Task DeleteEntityAsync_ReturnsConfirmation()
    {
        var (client, _) = CreateClient((_, _) =>
            Task.FromResult(JsonResponse(new { id = "alice", name = "Alice", edges_invalidated = 3 })));

        var elem = await client.DeleteEntityAsync("alice");
        Assert.Equal(3, elem.GetProperty("edges_invalidated").GetInt32());
    }

    [Fact]
    public async Task EntityEdgesAsync_HitsApiPath()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(Array.Empty<object>()));
        });
        await client.EntityEdgesAsync("alice");
        Assert.Equal("/api/entities/alice/edges", path);
    }

    [Fact]
    public async Task EdgeProvenanceAsync_HitsApiPath()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { edge_id = 42, supersedes = Array.Empty<object>() }));
        });
        await client.EdgeProvenanceAsync(42);
        Assert.Equal("/api/edges/42/provenance", path);
    }

    // ── Maintain / graphs ──

    [Fact]
    public async Task MaintainAsync_PostsToApiMaintain()
    {
        string? path = null;
        HttpMethod? method = null;
        var (client, _) = CreateClient((req, _) =>
        {
            method = req.Method;
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { ok = true }));
        });
        await client.MaintainAsync();
        Assert.Equal(HttpMethod.Post, method);
        Assert.Equal("/api/maintain", path);
    }

    [Fact]
    public async Task ListGraphsAsync_HitsApiGraphs()
    {
        string? path = null;
        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { @default = "default", graphs = new[] { "default" } }));
        });
        var result = await client.ListGraphsAsync();
        Assert.Equal("/api/graphs", path);
        Assert.Equal("default", result.Default);
    }

    [Fact]
    public async Task DropGraphAsync_HitsApiGraphsDrop()
    {
        string? path = null;
        HttpMethod? method = null;
        var (client, _) = CreateClient((req, _) =>
        {
            method = req.Method;
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new { ok = true }));
        });
        await client.DropGraphAsync("other");
        Assert.Equal(HttpMethod.Delete, method);
        Assert.Equal("/api/graphs/drop/other", path);
    }

    [Fact]
    public async Task AuditAsync_BuildsQueryString()
    {
        string? query = null;
        var (client, _) = CreateClient((req, _) =>
        {
            query = req.RequestUri?.Query;
            return Task.FromResult(JsonResponse(new { entries = Array.Empty<object>() }));
        });
        await client.AuditAsync(userId: "alice", limit: 10);
        Assert.Contains("user_id=alice", query);
        Assert.Contains("limit=10", query);
    }

    // ── Admin ──

    [Fact]
    public async Task CreateUserAsync_PostsAndReturnsApiKey()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                user_id = "u1",
                api_key = "secret-key",
            }));
        });

        var result = await client.CreateUserAsync(new CreateUserRequest
        {
            UserId = "u1",
            DisplayName = "User One",
            Role = "admin",
            Graphs = ["g1"],
        });

        Assert.Equal("/api/admin/users", path);
        Assert.Equal("u1", result.UserId);
        Assert.Equal("secret-key", result.ApiKey);
    }

    [Fact]
    public async Task ListUsersAsync_ReturnsUserList()
    {
        var (client, _) = CreateClient((req, _) =>
            Task.FromResult(JsonResponse(new
            {
                users = new[]
                {
                    new { user_id = "u1", display_name = "User One", role = "admin", graphs = new[] { "g1" }, key_count = 2 },
                },
            })));

        var result = await client.ListUsersAsync();
        Assert.Single(result.Users!);
        Assert.Equal("u1", result.Users![0].UserId);
        Assert.Equal(2, result.Users![0].KeyCount);
    }

    [Fact]
    public async Task DeleteUserAsync_SendsDeleteRequest()
    {
        HttpMethod? method = null;
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            method = req.Method;
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.NoContent));
        });

        await client.DeleteUserAsync("u1");
        Assert.Equal(HttpMethod.Delete, method);
        Assert.Equal("/api/admin/users/u1", path);
    }

    [Fact]
    public async Task CreateKeyAsync_PostsLabelAndReturnsKey()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                user_id = "u1",
                label = "my-key",
                api_key = "new-secret",
            }));
        });

        var result = await client.CreateKeyAsync("u1", new CreateKeyRequest { Label = "my-key" });
        Assert.Equal("/api/admin/users/u1/keys", path);
        Assert.Equal("new-secret", result.ApiKey);
    }

    [Fact]
    public async Task ListKeysAsync_ReturnsKeyList()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                keys = new[]
                {
                    new { label = "k1", created_at = "2025-01-01T00:00:00Z" },
                },
            }));
        });

        var result = await client.ListKeysAsync("u1");
        Assert.Equal("/api/admin/users/u1/keys", path);
        Assert.Single(result.Keys!);
        Assert.Equal("k1", result.Keys![0].Label);
    }

    [Fact]
    public async Task RevokeKeyAsync_SendsDeleteRequest()
    {
        HttpMethod? method = null;
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            method = req.Method;
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.NoContent));
        });

        await client.RevokeKeyAsync("u1", "k1");
        Assert.Equal(HttpMethod.Delete, method);
        Assert.Equal("/api/admin/users/u1/keys/k1", path);
    }

    // ── Error mapping ──

    [Theory]
    [InlineData(HttpStatusCode.Unauthorized, typeof(AuthenticationException))]
    [InlineData(HttpStatusCode.Forbidden, typeof(ForbiddenException))]
    [InlineData((HttpStatusCode)429, typeof(RateLimitException))]
    [InlineData(HttpStatusCode.InternalServerError, typeof(HippoException))]
    public async Task ErrorStatusCodes_MapToCorrectExceptions(HttpStatusCode status, Type expected)
    {
        var (client, _) = CreateClient((_, _) =>
            Task.FromResult(new HttpResponseMessage(status)
            {
                Content = new StringContent("error body"),
            }));

        var ex = await Assert.ThrowsAsync(expected, () => client.HealthAsync());
        var hippoEx = Assert.IsAssignableFrom<HippoException>(ex);
        Assert.Equal((int)status, hippoEx.StatusCode);
        Assert.Equal("error body", hippoEx.Message);
    }

    // ── Dispose ──

    [Fact]
    public void Dispose_WhenOwningHttpClient_DisposesIt()
    {
        var client = new HippoClient("http://localhost:21693", "key");
        client.Dispose();
    }

    // ── Optional fields omitted from JSON ──

    [Fact]
    public async Task RememberAsync_OmitsNullOptionalFields()
    {
        string? body = null;

        var (client, _) = CreateClient(async (req, _) =>
        {
            body = await req.Content!.ReadAsStringAsync();
            return JsonResponse(SampleRemember());
        });

        await client.RememberAsync(new RememberRequest { Statement = "hello" });

        Assert.DoesNotContain("source_agent", body);
        Assert.DoesNotContain("source_credibility_hint", body);
        Assert.DoesNotContain("graph", body);
        Assert.DoesNotContain("ttl_secs", body);
    }

    // ── Retry with exponential backoff ──

    [Fact]
    public async Task Retry_RetriesOn502ThenSucceeds()
    {
        int attempts = 0;

        var (client, _) = CreateClient((_, _) =>
        {
            attempts++;
            if (attempts == 1)
                return Task.FromResult(new HttpResponseMessage(HttpStatusCode.BadGateway)
                {
                    Content = new StringContent("bad gateway"),
                });
            return Task.FromResult(JsonResponse(new { status = "ok", graph = "default" }));
        }, maxRetries: 3);

        var result = await client.HealthAsync();
        Assert.Equal("ok", result.Status);
        Assert.Equal(2, attempts);
    }

    [Fact]
    public async Task Retry_RespectsRetryAfterHeaderInSeconds()
    {
        int attempts = 0;
        var sw = System.Diagnostics.Stopwatch.StartNew();

        var (client, _) = CreateClient((_, _) =>
        {
            attempts++;
            if (attempts == 1)
            {
                var resp = new HttpResponseMessage((HttpStatusCode)429)
                {
                    Content = new StringContent("rate limited"),
                };
                resp.Headers.RetryAfter = new System.Net.Http.Headers.RetryConditionHeaderValue(TimeSpan.FromSeconds(1));
                return Task.FromResult(resp);
            }
            return Task.FromResult(JsonResponse(new { status = "ok", graph = "default" }));
        }, maxRetries: 3);

        await client.HealthAsync();
        sw.Stop();

        Assert.Equal(2, attempts);
        Assert.True(sw.Elapsed >= TimeSpan.FromMilliseconds(900),
            $"Expected at least 900ms delay, got {sw.ElapsedMilliseconds}ms");
    }

    [Fact]
    public async Task Retry_MaxRetriesZeroDisablesRetry()
    {
        int attempts = 0;

        var (client, _) = CreateClient((_, _) =>
        {
            attempts++;
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.BadGateway)
            {
                Content = new StringContent("bad gateway"),
            });
        }, maxRetries: 0);

        var ex = await Assert.ThrowsAsync<HippoException>(() => client.HealthAsync());
        Assert.Equal(502, ex.StatusCode);
        Assert.Equal(1, attempts);
    }

    [Fact]
    public async Task Retry_ExhaustsMaxRetriesAndThrows()
    {
        int attempts = 0;

        var (client, _) = CreateClient((_, _) =>
        {
            attempts++;
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.ServiceUnavailable)
            {
                Content = new StringContent("unavailable"),
            });
        }, maxRetries: 2);

        var ex = await Assert.ThrowsAsync<HippoException>(() => client.HealthAsync());
        Assert.Equal(503, ex.StatusCode);
        Assert.Equal(3, attempts);
    }

    [Fact]
    public async Task Retry_429SetsRetryAfterOnException_WhenRetriesExhausted()
    {
        var (client, _) = CreateClient((_, _) =>
        {
            var resp = new HttpResponseMessage((HttpStatusCode)429)
            {
                Content = new StringContent("rate limited"),
            };
            resp.Headers.RetryAfter = new System.Net.Http.Headers.RetryConditionHeaderValue(TimeSpan.FromSeconds(60));
            return Task.FromResult(resp);
        }, maxRetries: 0);

        var ex = await Assert.ThrowsAsync<RateLimitException>(() => client.HealthAsync());
        Assert.NotNull(ex.RetryAfter);
        Assert.True(ex.RetryAfter!.Value.TotalSeconds >= 59);
    }

    [Fact]
    public async Task Retry_NonRetryableStatusNotRetried()
    {
        int attempts = 0;

        var (client, _) = CreateClient((_, _) =>
        {
            attempts++;
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.BadRequest)
            {
                Content = new StringContent("bad request"),
            });
        }, maxRetries: 3);

        var ex = await Assert.ThrowsAsync<HippoException>(() => client.HealthAsync());
        Assert.Equal(400, ex.StatusCode);
        Assert.Equal(1, attempts);
    }

    // ── SSE streaming ──

    [Fact]
    public async Task EventsAsync_ParsesSseStream()
    {
        var sseContent = "event: entity_created\ndata: {\"name\":\"Alice\"}\n\nevent: edge_created\ndata: {\"source\":\"Alice\",\"target\":\"Bob\"}\n\n";

        var (client, _) = CreateClient((_, _) =>
        {
            var resp = new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StringContent(sseContent, Encoding.UTF8, "text/event-stream"),
            };
            return Task.FromResult(resp);
        });

        var events = new List<GraphEvent>();
        await foreach (var evt in client.EventsAsync())
        {
            events.Add(evt);
        }

        Assert.Equal(2, events.Count);
        Assert.Equal("entity_created", events[0].Event);
        Assert.Contains("Alice", events[0].Data!);
        Assert.Equal("edge_created", events[1].Event);
        Assert.Contains("Bob", events[1].Data!);
    }

    [Fact]
    public async Task EventsAsync_HandlesTrailingEventWithoutBlankLine()
    {
        var sseContent = "event: update\ndata: {\"id\":1}";

        var (client, _) = CreateClient((_, _) =>
        {
            var resp = new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StringContent(sseContent, Encoding.UTF8, "text/event-stream"),
            };
            return Task.FromResult(resp);
        });

        var events = new List<GraphEvent>();
        await foreach (var evt in client.EventsAsync())
        {
            events.Add(evt);
        }

        Assert.Single(events);
        Assert.Equal("update", events[0].Event);
    }

    [Fact]
    public async Task EventsAsync_HitsApiEventsAndPassesGraphParam()
    {
        string? capturedUri = null;

        var (client, _) = CreateClient((req, _) =>
        {
            capturedUri = req.RequestUri?.ToString();
            var resp = new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StringContent("", Encoding.UTF8, "text/event-stream"),
            };
            return Task.FromResult(resp);
        });

        await foreach (var _ in client.EventsAsync(graph: "myGraph")) { }

        Assert.Contains("/api/events?graph=myGraph", capturedUri!);
    }

    // ── Response extension methods ──

    [Fact]
    public void FindSubject_ReturnsMatchingFact()
    {
        var response = new ContextResponse
        {
            Facts = [
                new ContextFact { Fact = "f", Subject = "Alice", RelationType = "x", Object = "y", ValidAt = "t", MemoryTier = "long_term" },
                new ContextFact { Fact = "f", Subject = "Bob", RelationType = "x", Object = "y", ValidAt = "t", MemoryTier = "long_term" },
            ],
        };

        var fact = response.FindSubject("alice");
        Assert.NotNull(fact);
        Assert.Equal("Alice", fact!.Subject);
    }

    [Fact]
    public void FindSubject_ReturnsNullWhenNotFound()
    {
        var response = new ContextResponse
        {
            Facts = [new ContextFact { Fact = "f", Subject = "Alice", RelationType = "x", Object = "y", ValidAt = "t", MemoryTier = "long_term" }],
        };

        Assert.Null(response.FindSubject("Charlie"));
    }

    [Fact]
    public void FactsAbout_FiltersFactsInvolvingEntity()
    {
        var response = new ContextResponse
        {
            Facts = [
                new ContextFact { Fact = "f1", Subject = "Alice", RelationType = "knows", Object = "Bob", ValidAt = "t", MemoryTier = "long_term" },
                new ContextFact { Fact = "f2", Subject = "Charlie", RelationType = "knows", Object = "Diana", ValidAt = "t", MemoryTier = "long_term" },
                new ContextFact { Fact = "f3", Subject = "Bob", RelationType = "trusts", Object = "Alice", ValidAt = "t", MemoryTier = "long_term" },
            ],
        };

        var facts = response.FactsAbout("Alice").ToList();
        Assert.Equal(2, facts.Count);
        Assert.All(facts, f =>
            Assert.True(f.Subject == "Alice" || f.Object == "Alice"));
    }

    [Fact]
    public void IsDuplicate_TrueWhenFactsWrittenIsZero()
    {
        var response = new RememberResponse { FactsWritten = 0 };
        Assert.True(response.IsDuplicate());
    }

    [Fact]
    public void IsDuplicate_FalseWhenFactsWritten()
    {
        var response = new RememberResponse { FactsWritten = 1 };
        Assert.False(response.IsDuplicate());
    }

    [Fact]
    public void Failures_ReturnsEmptyWhenAllSucceeded()
    {
        var response = new RememberBatchResponse
        {
            Total = 2,
            Succeeded = 2,
            Failed = 0,
            Results = [
                new BatchRememberResult { Statement = "a", Ok = true },
                new BatchRememberResult { Statement = "b", Ok = true },
            ],
        };

        Assert.Empty(response.Failures());
    }

    [Fact]
    public void Failures_ReturnsFailedResults()
    {
        var response = new RememberBatchResponse
        {
            Total = 2,
            Succeeded = 1,
            Failed = 1,
            Results = [
                new BatchRememberResult { Statement = "a", Ok = true },
                new BatchRememberResult { Statement = "b", Ok = false, Error = "oops" },
            ],
        };

        var failures = response.Failures().ToList();
        Assert.Single(failures);
        Assert.Equal("oops", failures[0].Error);
    }

    // ── Logger ──

    [Fact]
    public async Task Logger_ReceivesDebugAndWarnMessages()
    {
        var debugMessages = new List<string>();
        var warnMessages = new List<string>();
        var logger = new TestLogger(debugMessages, warnMessages);

        int attempts = 0;
        var (client, _) = CreateClient((_, _) =>
        {
            attempts++;
            if (attempts == 1)
                return Task.FromResult(new HttpResponseMessage(HttpStatusCode.BadGateway)
                {
                    Content = new StringContent("bad gateway"),
                });
            return Task.FromResult(JsonResponse(new { status = "ok", graph = "default" }));
        }, maxRetries: 3, logger: logger);

        await client.HealthAsync();

        Assert.True(debugMessages.Count >= 2, $"Expected at least 2 debug messages, got {debugMessages.Count}");
        Assert.Single(warnMessages);
        Assert.Contains("Retry", warnMessages[0]);
    }

    [Fact]
    public async Task Logger_NullLoggerDoesNotThrow()
    {
        var (client, _) = CreateClient((_, _) =>
            Task.FromResult(JsonResponse(new { status = "ok", graph = "default" })),
            logger: null);

        var result = await client.HealthAsync();
        Assert.Equal("ok", result.Status);
    }

    private sealed class TestLogger : IHippoLogger
    {
        private readonly List<string> _debug;
        private readonly List<string> _warn;

        public TestLogger(List<string> debug, List<string> warn)
        {
            _debug = debug;
            _warn = warn;
        }

        public void Debug(string message) => _debug.Add(message);
        public void Warn(string message) => _warn.Add(message);
    }
}
