# hippo-sdk (Go)

Go SDK for the [Hippo](https://github.com/dcprevere/hippo) natural-language database.

Zero external dependencies -- uses only the Go standard library.

## Install

```
go get github.com/dcprevere/hippo/sdks/go
```

Requires Go 1.21 or later.

## Quick start

```go
package main

import (
    "context"
    "fmt"
    "log"

    hippo "github.com/dcprevere/hippo/sdks/go"
)

func main() {
    client := hippo.NewClient("http://localhost:3000", hippo.WithAPIKey("your-api-key"))
    ctx := context.Background()

    // Store a fact
    resp, err := client.Remember(ctx, &hippo.RememberRequest{
        Statement: "Alice works at Acme Corp",
    })
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Stored %d facts\n", resp.FactsWritten)

    // Ask a question
    answer, err := client.Ask(ctx, &hippo.AskRequest{
        Question: "Where does Alice work?",
    })
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println(answer.Answer)
}
```

## Usage

### Creating a client

```go
// Minimal
client := hippo.NewClient("http://localhost:3000")

// With options
client := hippo.NewClient("http://localhost:3000",
    hippo.WithAPIKey("your-api-key"),
    hippo.WithHTTPClient(&http.Client{Timeout: 10 * time.Second}),
)
```

### Storing facts

```go
// Single statement
resp, err := client.Remember(ctx, &hippo.RememberRequest{
    Statement: "Bob is Alice's manager",
})

// Batch
graph := "team"
batch, err := client.RememberBatch(ctx, &hippo.BatchRememberRequest{
    Statements: []string{
        "Alice joined in 2020",
        "Bob joined in 2018",
    },
    Graph: &graph,
})
fmt.Printf("%d/%d succeeded\n", batch.Succeeded, batch.Total)
```

### Querying

```go
// Get graph context
ctxResp, err := client.Context(ctx, &hippo.ContextRequest{
    Query: "Alice",
})
for _, node := range ctxResp.Nodes {
    fmt.Println(node.Label)
}

// Ask a question
answer, err := client.Ask(ctx, &hippo.AskRequest{
    Question: "Who manages Alice?",
})
fmt.Println(answer.Answer)
```

### Admin operations

```go
// Create a user
user, err := client.CreateUser(ctx, &hippo.CreateUserRequest{
    UserID:      "alice",
    DisplayName: "Alice",
})
fmt.Println("API key:", user.APIKey)

// List users
users, err := client.ListUsers(ctx)

// Manage API keys
key, err := client.CreateKey(ctx, "alice", &hippo.CreateKeyRequest{Label: "ci"})
keys, err := client.ListKeys(ctx, "alice")
err = client.DeleteKey(ctx, "alice", "ci")

// Delete user
err = client.DeleteUser(ctx, "alice")
```

### Health check

```go
health, err := client.Health(ctx) // no auth required
fmt.Println(health.Status)
```

### Error handling

```go
_, err := client.Remember(ctx, &hippo.RememberRequest{Statement: "test"})
if err != nil {
    var he *hippo.HippoError
    if errors.As(err, &he) {
        if he.IsAuthError() {
            // 401
        }
        if he.IsForbidden() {
            // 403
        }
        if he.IsRateLimited() {
            // 429
        }
        fmt.Printf("HTTP %d: %s\n", he.StatusCode, he.Message)
    }
}
```

## Thread safety

The `Client` is safe for concurrent use from multiple goroutines.
