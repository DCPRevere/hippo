using System.Net;
using System.Text;
using System.Text.Json;

namespace Hippo.Sdk.Tests;

/// <summary>
/// A delegating handler that lets tests supply a lambda to handle requests.
/// </summary>
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

    // ── Auth header ──

    [Fact]
    public async Task BearerTokenIsSentInAuthorizationHeader()
    {
        string? captured = null;
        var (client, _) = CreateClient((req, _) =>
        {
            captured = req.Headers.Authorization?.ToString();
            return Task.FromResult(JsonResponse(new { status = "ok" }));
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
            return Task.FromResult(JsonResponse(new { status = "ok" }));
        }, apiKey: null);

        await client.HealthAsync();
        Assert.False(hadAuth);
    }

    // ── Health ──

    [Fact]
    public async Task HealthAsync_ReturnsStatus()
    {
        var (client, _) = CreateClient((_, _) =>
            Task.FromResult(JsonResponse(new { status = "healthy", graph = "default" })));

        var result = await client.HealthAsync();
        Assert.Equal("healthy", result.Status);
        Assert.Equal("default", result.Graph);
    }

    // ── Remember ──

    [Fact]
    public async Task RememberAsync_PostsStatementAndReturnsResponse()
    {
        HttpMethod? method = null;
        string? path = null;
        string? body = null;

        var (client, _) = CreateClient(async (req, _) =>
        {
            method = req.Method;
            path = req.RequestUri?.AbsolutePath;
            body = await req.Content!.ReadAsStringAsync();
            return JsonResponse(new
            {
                entities_created = 1,
                entities_resolved = 2,
                facts_written = 3,
                contradictions_invalidated = 0,
            });
        });

        var result = await client.RememberAsync(new RememberRequest
        {
            Statement = "Alice likes Bob",
            SourceAgent = "test",
            Graph = "g1",
            TtlSecs = 600,
        });

        Assert.Equal(HttpMethod.Post, method);
        Assert.Equal("/remember", path);
        Assert.Contains("\"statement\":\"Alice likes Bob\"", body);
        Assert.Contains("\"source_agent\":\"test\"", body);
        Assert.Contains("\"graph\":\"g1\"", body);
        Assert.Contains("\"ttl_secs\":600", body);
        Assert.Equal(1, result.EntitiesCreated);
        Assert.Equal(2, result.EntitiesResolved);
        Assert.Equal(3, result.FactsWritten);
        Assert.Equal(0, result.ContradictionsInvalidated);
    }

    // ── Remember Batch ──

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
                    new { entities_created = 1, entities_resolved = 0, facts_written = 1, contradictions_invalidated = 0 },
                    new { entities_created = 0, entities_resolved = 1, facts_written = 1, contradictions_invalidated = 0 },
                },
            });
        });

        var result = await client.RememberBatchAsync(new RememberBatchRequest
        {
            Statements = ["A is B", "C is D"],
            Parallel = true,
        });

        Assert.Equal("/remember/batch", path);
        Assert.Equal(2, result.Total);
        Assert.Equal(2, result.Succeeded);
        Assert.Equal(0, result.Failed);
        Assert.Equal(2, result.Results!.Length);
    }

    // ── Context ──

    [Fact]
    public async Task ContextAsync_PostsQueryAndReturnsGraph()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                nodes = new[] { new { name = "Alice", entity_type = "person" } },
                edges = new[] { new { source = "Alice", target = "Bob", relation = "likes" } },
            }));
        });

        var result = await client.ContextAsync(new ContextRequest
        {
            Query = "Alice",
            Limit = 10,
            MaxHops = 2,
        });

        Assert.Equal("/context", path);
        Assert.Single(result.Nodes!);
        Assert.Equal("Alice", result.Nodes![0].Name);
        Assert.Single(result.Edges!);
        Assert.Equal("likes", result.Edges![0].Relation);
    }

    // ── Ask ──

    [Fact]
    public async Task AskAsync_PostsQuestionAndReturnsAnswer()
    {
        string? path = null;

        var (client, _) = CreateClient((req, _) =>
        {
            path = req.RequestUri?.AbsolutePath;
            return Task.FromResult(JsonResponse(new
            {
                answer = "Bob",
                facts = new[] { "Alice likes Bob" },
            }));
        });

        var result = await client.AskAsync(new AskRequest
        {
            Question = "Who does Alice like?",
            Verbose = true,
        });

        Assert.Equal("/ask", path);
        Assert.Equal("Bob", result.Answer);
        Assert.Single(result.Facts!);
    }

    // ── Admin: Create User ──

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

        Assert.Equal("/admin/users", path);
        Assert.Equal("u1", result.UserId);
        Assert.Equal("secret-key", result.ApiKey);
    }

    // ── Admin: List Users ──

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

    // ── Admin: Delete User ──

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
        Assert.Equal("/admin/users/u1", path);
    }

    // ── Admin: Create Key ──

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
        Assert.Equal("/admin/users/u1/keys", path);
        Assert.Equal("new-secret", result.ApiKey);
    }

    // ── Admin: List Keys ──

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
        Assert.Equal("/admin/users/u1/keys", path);
        Assert.Single(result.Keys!);
        Assert.Equal("k1", result.Keys![0].Label);
    }

    // ── Admin: Revoke Key ──

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
        Assert.Equal("/admin/users/u1/keys/k1", path);
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
        // When no HttpClient is provided, HippoClient creates and owns one.
        // We just verify Dispose doesn't throw.
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
            return JsonResponse(new
            {
                entities_created = 0,
                entities_resolved = 0,
                facts_written = 0,
                contradictions_invalidated = 0,
            });
        });

        await client.RememberAsync(new RememberRequest { Statement = "hello" });

        Assert.DoesNotContain("source_agent", body);
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
            return Task.FromResult(JsonResponse(new { status = "ok" }));
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
            return Task.FromResult(JsonResponse(new { status = "ok" }));
        }, maxRetries: 3);

        await client.HealthAsync();
        sw.Stop();

        Assert.Equal(2, attempts);
        // Should have waited at least ~1 second from Retry-After
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
        // 1 initial + 2 retries = 3 attempts
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
    public async Task EventsAsync_PassesGraphQueryParam()
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

        Assert.Contains("events?graph=myGraph", capturedUri!);
    }

    // ── Response extension methods ──

    [Fact]
    public void FindNode_ReturnsMatchingNode()
    {
        var response = new ContextResponse
        {
            Nodes = [
                new ContextNode { Name = "Alice", EntityType = "person" },
                new ContextNode { Name = "Bob", EntityType = "person" },
            ],
        };

        var node = response.FindNode("alice"); // case-insensitive
        Assert.NotNull(node);
        Assert.Equal("Alice", node!.Name);
    }

    [Fact]
    public void FindNode_ReturnsNullWhenNotFound()
    {
        var response = new ContextResponse
        {
            Nodes = [new ContextNode { Name = "Alice" }],
        };

        Assert.Null(response.FindNode("Charlie"));
    }

    [Fact]
    public void FactsAbout_FiltersEdgesInvolvingEntity()
    {
        var response = new ContextResponse
        {
            Edges = [
                new ContextEdge { Source = "Alice", Target = "Bob", Relation = "likes" },
                new ContextEdge { Source = "Charlie", Target = "Diana", Relation = "knows" },
                new ContextEdge { Source = "Bob", Target = "Alice", Relation = "trusts" },
            ],
        };

        var facts = response.FactsAbout("Alice").ToList();
        Assert.Equal(2, facts.Count);
        Assert.All(facts, f =>
            Assert.True(f.Source == "Alice" || f.Target == "Alice"));
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
                new RememberResponse { EntitiesCreated = 1, FactsWritten = 1 },
                new RememberResponse { EntitiesCreated = 1, FactsWritten = 1 },
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
                new RememberResponse { EntitiesCreated = 1, FactsWritten = 1 },
                new RememberResponse { EntitiesCreated = 0, EntitiesResolved = 0, FactsWritten = 0 },
            ],
        };

        var failures = response.Failures().ToList();
        Assert.Single(failures);
        Assert.Equal(0, failures[0].FactsWritten);
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
            return Task.FromResult(JsonResponse(new { status = "ok" }));
        }, maxRetries: 3, logger: logger);

        await client.HealthAsync();

        // Should have debug messages for requests and responses
        Assert.True(debugMessages.Count >= 2, $"Expected at least 2 debug messages, got {debugMessages.Count}");
        // Should have a warn message for the retry
        Assert.Single(warnMessages);
        Assert.Contains("Retry", warnMessages[0]);
    }

    [Fact]
    public async Task Logger_NullLoggerDoesNotThrow()
    {
        var (client, _) = CreateClient((_, _) =>
            Task.FromResult(JsonResponse(new { status = "ok" })),
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
