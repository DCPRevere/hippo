using System.Globalization;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Hippo.Sdk;

/// <summary>
/// Client for the Hippo natural-language database REST API.
/// </summary>
public sealed class HippoClient : IDisposable
{
    private readonly HttpClient _http;
    private readonly bool _ownsHttp;
    private readonly int _maxRetries;
    private readonly IHippoLogger? _logger;

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        PropertyNameCaseInsensitive = true,
    };

    private static readonly HashSet<HttpStatusCode> RetryableStatusCodes = new()
    {
        (HttpStatusCode)429,
        HttpStatusCode.BadGateway,
        HttpStatusCode.ServiceUnavailable,
        HttpStatusCode.GatewayTimeout,
    };

    /// <summary>
    /// Creates a new <see cref="HippoClient"/>.
    /// </summary>
    /// <param name="baseUrl">
    /// Base URL of the Hippo server. Falls back to the <c>HIPPO_URL</c> environment variable,
    /// then <c>http://localhost:21693</c>.
    /// </param>
    /// <param name="apiKey">
    /// Bearer token for authentication. Falls back to the <c>HIPPO_API_KEY</c> environment variable.
    /// </param>
    /// <param name="httpClient">
    /// Optional pre-configured <see cref="HttpClient"/>. When provided the caller is
    /// responsible for its lifetime; <see cref="Dispose"/> will not dispose it.
    /// </param>
    /// <param name="timeout">
    /// Request timeout. Defaults to 30 seconds. Only applied when the client owns the
    /// <see cref="HttpClient"/> (i.e. <paramref name="httpClient"/> is null).
    /// </param>
    /// <param name="maxRetries">
    /// Maximum number of retries for transient failures (429, 502, 503, 504).
    /// Set to 0 to disable retries. Defaults to 3.
    /// </param>
    /// <param name="logger">Optional logger for request/retry diagnostics.</param>
    public HippoClient(
        string? baseUrl = null,
        string? apiKey = null,
        HttpClient? httpClient = null,
        TimeSpan? timeout = null,
        int maxRetries = 3,
        IHippoLogger? logger = null)
    {
        baseUrl ??= Environment.GetEnvironmentVariable("HIPPO_URL") ?? "http://localhost:21693";
        apiKey ??= Environment.GetEnvironmentVariable("HIPPO_API_KEY");

        _ownsHttp = httpClient is null;
        _http = httpClient ?? new HttpClient();
        _maxRetries = maxRetries;
        _logger = logger;

        _http.BaseAddress ??= new Uri(baseUrl.TrimEnd('/') + "/");

        if (_ownsHttp)
        {
            _http.Timeout = timeout ?? TimeSpan.FromSeconds(30);
        }

        if (apiKey is not null)
        {
            _http.DefaultRequestHeaders.Authorization =
                new AuthenticationHeaderValue("Bearer", apiKey);
        }
    }

    // ── Core ──

    public Task<RememberResponse> RememberAsync(
        RememberRequest request, CancellationToken ct = default)
        => PostAsync<RememberRequest, RememberResponse>("remember", request, ct);

    public Task<RememberBatchResponse> RememberBatchAsync(
        RememberBatchRequest request, CancellationToken ct = default)
        => PostAsync<RememberBatchRequest, RememberBatchResponse>("remember/batch", request, ct);

    public Task<ContextResponse> ContextAsync(
        ContextRequest request, CancellationToken ct = default)
        => PostAsync<ContextRequest, ContextResponse>("context", request, ct);

    public Task<AskResponse> AskAsync(
        AskRequest request, CancellationToken ct = default)
        => PostAsync<AskRequest, AskResponse>("ask", request, ct);

    // ── Admin ──

    public Task<CreateUserResponse> CreateUserAsync(
        CreateUserRequest request, CancellationToken ct = default)
        => PostAsync<CreateUserRequest, CreateUserResponse>("admin/users", request, ct);

    public Task<ListUsersResponse> ListUsersAsync(CancellationToken ct = default)
        => GetAsync<ListUsersResponse>("admin/users", ct);

    public Task DeleteUserAsync(string userId, CancellationToken ct = default)
        => DeleteAsync($"admin/users/{Uri.EscapeDataString(userId)}", ct);

    public Task<CreateKeyResponse> CreateKeyAsync(
        string userId, CreateKeyRequest request, CancellationToken ct = default)
        => PostAsync<CreateKeyRequest, CreateKeyResponse>(
            $"admin/users/{Uri.EscapeDataString(userId)}/keys", request, ct);

    public Task<ListKeysResponse> ListKeysAsync(string userId, CancellationToken ct = default)
        => GetAsync<ListKeysResponse>($"admin/users/{Uri.EscapeDataString(userId)}/keys", ct);

    public Task RevokeKeyAsync(string userId, string label, CancellationToken ct = default)
        => DeleteAsync(
            $"admin/users/{Uri.EscapeDataString(userId)}/keys/{Uri.EscapeDataString(label)}", ct);

    // ── Observability ──

    public Task<HealthResponse> HealthAsync(CancellationToken ct = default)
        => GetAsync<HealthResponse>("health", ct);

    // ── SSE streaming ──

    /// <summary>
    /// Opens a streaming connection to the /events endpoint and yields
    /// <see cref="GraphEvent"/> records as they arrive.
    /// </summary>
    public async IAsyncEnumerable<GraphEvent> EventsAsync(
        string? graph = null,
        [EnumeratorCancellation] CancellationToken ct = default)
    {
        var path = graph is null ? "events" : $"events?graph={Uri.EscapeDataString(graph)}";

        var request = new HttpRequestMessage(HttpMethod.Get, path);
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("text/event-stream"));

        _logger?.Debug($"SSE GET {path}");

        var response = await _http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, ct)
            .ConfigureAwait(false);

        await EnsureSuccessAsync(response, ct).ConfigureAwait(false);

        using (response)
        {
            await using var stream = await response.Content.ReadAsStreamAsync(ct)
                .ConfigureAwait(false);
            using var reader = new StreamReader(stream);

            string? eventType = null;
            string? data = null;

            while (!ct.IsCancellationRequested)
            {
                var line = await reader.ReadLineAsync(ct).ConfigureAwait(false);
                if (line is null) break; // end of stream

                if (line.StartsWith("event:", StringComparison.Ordinal))
                {
                    eventType = line["event:".Length..].Trim();
                }
                else if (line.StartsWith("data:", StringComparison.Ordinal))
                {
                    data = line["data:".Length..].Trim();
                }
                else if (line.Length == 0)
                {
                    // Empty line signals end of an SSE event.
                    if (eventType is not null || data is not null)
                    {
                        yield return new GraphEvent { Event = eventType, Data = data };
                        eventType = null;
                        data = null;
                    }
                }
            }

            // Yield any trailing event without a final blank line.
            if (eventType is not null || data is not null)
            {
                yield return new GraphEvent { Event = eventType, Data = data };
            }
        }
    }

    // ── HTTP helpers ──

    private async Task<TResponse> PostAsync<TRequest, TResponse>(
        string path, TRequest body, CancellationToken ct)
    {
        using var response = await SendAsync(
            () =>
            {
                var req = new HttpRequestMessage(HttpMethod.Post, path)
                {
                    Content = JsonContent.Create(body, options: JsonOptions),
                };
                return req;
            },
            ct).ConfigureAwait(false);

        return (await response.Content.ReadFromJsonAsync<TResponse>(JsonOptions, ct)
            .ConfigureAwait(false))!;
    }

    private async Task<TResponse> GetAsync<TResponse>(string path, CancellationToken ct)
    {
        using var response = await SendAsync(
            () => new HttpRequestMessage(HttpMethod.Get, path),
            ct).ConfigureAwait(false);

        return (await response.Content.ReadFromJsonAsync<TResponse>(JsonOptions, ct)
            .ConfigureAwait(false))!;
    }

    private async Task DeleteAsync(string path, CancellationToken ct)
    {
        using var response = await SendAsync(
            () => new HttpRequestMessage(HttpMethod.Delete, path),
            ct).ConfigureAwait(false);
    }

    /// <summary>
    /// Sends a request with retry and exponential backoff for transient failures.
    /// The <paramref name="requestFactory"/> is called on each attempt because
    /// <see cref="HttpRequestMessage"/> cannot be reused after it has been sent.
    /// </summary>
    private async Task<HttpResponseMessage> SendAsync(
        Func<HttpRequestMessage> requestFactory,
        CancellationToken ct)
    {
        const double baseDelaySecs = 0.5;

        for (int attempt = 0; ; attempt++)
        {
            using var request = requestFactory();

            _logger?.Debug($"{request.Method} {request.RequestUri}");

            var response = await _http.SendAsync(request, ct).ConfigureAwait(false);

            _logger?.Debug($"Status {(int)response.StatusCode} for {request.Method} {request.RequestUri}");

            if (response.IsSuccessStatusCode)
                return response;

            // Check if retryable
            bool retryable = RetryableStatusCodes.Contains(response.StatusCode) && attempt < _maxRetries;

            if (!retryable)
            {
                await EnsureSuccessAsync(response, ct).ConfigureAwait(false);
                // EnsureSuccessAsync always throws for non-success; this is unreachable.
                return response;
            }

            // Determine delay
            TimeSpan delay = GetRetryDelay(response, attempt, baseDelaySecs);

            _logger?.Warn($"Retry {attempt + 1}/{_maxRetries} after {delay.TotalMilliseconds:F0}ms " +
                          $"(status {(int)response.StatusCode})");

            response.Dispose();

            await Task.Delay(delay, ct).ConfigureAwait(false);
        }
    }

    private static TimeSpan GetRetryDelay(HttpResponseMessage response, int attempt, double baseDelaySecs)
    {
        // Try Retry-After header first
        TimeSpan? retryAfter = ParseRetryAfter(response);
        if (retryAfter.HasValue)
            return retryAfter.Value;

        // Exponential backoff with jitter
        double delaySecs = baseDelaySecs * Math.Pow(2, attempt);
        double jitter = Random.Shared.NextDouble() * delaySecs * 0.5;
        return TimeSpan.FromSeconds(delaySecs + jitter);
    }

    internal static TimeSpan? ParseRetryAfter(HttpResponseMessage response)
    {
        if (response.Headers.RetryAfter is null)
            return null;

        // Seconds form
        if (response.Headers.RetryAfter.Delta.HasValue)
            return response.Headers.RetryAfter.Delta.Value;

        // HTTP-date form
        if (response.Headers.RetryAfter.Date.HasValue)
        {
            var delay = response.Headers.RetryAfter.Date.Value - DateTimeOffset.UtcNow;
            return delay > TimeSpan.Zero ? delay : TimeSpan.Zero;
        }

        return null;
    }

    private static async Task EnsureSuccessAsync(HttpResponseMessage response, CancellationToken ct)
    {
        if (response.IsSuccessStatusCode) return;

        var body = await response.Content.ReadAsStringAsync(ct).ConfigureAwait(false);
        var status = (int)response.StatusCode;

        throw status switch
        {
            401 => new AuthenticationException(body),
            403 => new ForbiddenException(body),
            429 => new RateLimitException(body, ParseRetryAfter(response)),
            _ => new HippoException(body, status),
        };
    }

    public void Dispose()
    {
        if (_ownsHttp) _http.Dispose();
    }
}
