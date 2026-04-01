# Hippo .NET SDK

A .NET 8 client library for the Hippo natural-language database REST API. Zero external dependencies -- uses only `System.Net.Http` and `System.Text.Json`.

## Installation

Add a project reference to `Hippo.Sdk` or (when published) install the NuGet package.

## Quick start

```csharp
using Hippo.Sdk;

using var client = new HippoClient(
    baseUrl: "http://localhost:21693",
    apiKey: "your-api-key");

// Store a fact
var result = await client.RememberAsync(new RememberRequest
{
    Statement = "Alice works at Acme Corp",
});

// Query context
var context = await client.ContextAsync(new ContextRequest
{
    Query = "Alice",
    Limit = 10,
});

// Ask a question
var answer = await client.AskAsync(new AskRequest
{
    Question = "Where does Alice work?",
});
Console.WriteLine(answer.Answer);
```

## Configuration

The constructor reads defaults from environment variables when arguments are omitted:

| Parameter | Env var | Default |
|-----------|---------|---------|
| `baseUrl` | `HIPPO_URL` | `http://localhost:21693` |
| `apiKey` | `HIPPO_API_KEY` | `null` |

You can also supply your own `HttpClient` for connection pooling or custom handlers:

```csharp
var http = new HttpClient();
using var client = new HippoClient(httpClient: http);
```

When you provide an `HttpClient`, the SDK will not dispose it.

## API methods

### Core

- `RememberAsync(RememberRequest)` -- store a statement
- `RememberBatchAsync(RememberBatchRequest)` -- store multiple statements
- `ContextAsync(ContextRequest)` -- retrieve graph context for a query
- `AskAsync(AskRequest)` -- ask a natural-language question

### Admin

- `CreateUserAsync(CreateUserRequest)` -- create a user (returns API key)
- `ListUsersAsync()` -- list all users
- `DeleteUserAsync(userId)` -- delete a user
- `CreateKeyAsync(userId, CreateKeyRequest)` -- create an API key for a user
- `ListKeysAsync(userId)` -- list keys for a user
- `RevokeKeyAsync(userId, label)` -- revoke a key

### Observability

- `HealthAsync()` -- check server health (no auth required)

## Error handling

The SDK throws typed exceptions for HTTP error responses:

| Status | Exception |
|--------|-----------|
| 401 | `AuthenticationException` |
| 403 | `ForbiddenException` |
| 429 | `RateLimitException` |
| Other | `HippoException` |

All exceptions extend `HippoException`, which exposes a `StatusCode` property and the response body as the `Message`.

## Building

```bash
dotnet build sdks/dotnet/Hippo.Sdk.slnx
dotnet test sdks/dotnet/Hippo.Sdk.slnx
```
