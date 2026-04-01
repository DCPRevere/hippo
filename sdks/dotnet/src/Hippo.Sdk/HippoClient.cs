using System.Net.Http.Headers;
using System.Net.Http.Json;
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

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        PropertyNameCaseInsensitive = true,
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
    public HippoClient(string? baseUrl = null, string? apiKey = null, HttpClient? httpClient = null)
    {
        baseUrl ??= Environment.GetEnvironmentVariable("HIPPO_URL") ?? "http://localhost:21693";
        apiKey ??= Environment.GetEnvironmentVariable("HIPPO_API_KEY");

        _ownsHttp = httpClient is null;
        _http = httpClient ?? new HttpClient();

        _http.BaseAddress ??= new Uri(baseUrl.TrimEnd('/') + "/");

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

    // ── HTTP helpers ──

    private async Task<TResponse> PostAsync<TRequest, TResponse>(
        string path, TRequest body, CancellationToken ct)
    {
        using var response = await _http.PostAsJsonAsync(path, body, JsonOptions, ct)
            .ConfigureAwait(false);
        await EnsureSuccessAsync(response, ct).ConfigureAwait(false);
        return (await response.Content.ReadFromJsonAsync<TResponse>(JsonOptions, ct)
            .ConfigureAwait(false))!;
    }

    private async Task<TResponse> GetAsync<TResponse>(string path, CancellationToken ct)
    {
        using var response = await _http.GetAsync(path, ct).ConfigureAwait(false);
        await EnsureSuccessAsync(response, ct).ConfigureAwait(false);
        return (await response.Content.ReadFromJsonAsync<TResponse>(JsonOptions, ct)
            .ConfigureAwait(false))!;
    }

    private async Task DeleteAsync(string path, CancellationToken ct)
    {
        using var response = await _http.DeleteAsync(path, ct).ConfigureAwait(false);
        await EnsureSuccessAsync(response, ct).ConfigureAwait(false);
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
            429 => new RateLimitException(body),
            _ => new HippoException(body, status),
        };
    }

    public void Dispose()
    {
        if (_ownsHttp) _http.Dispose();
    }
}
