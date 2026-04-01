# @hippo-ai/sdk

TypeScript SDK for the Hippo natural-language database API.

Requires Node.js 18+ (uses native `fetch`). Zero runtime dependencies.

## Install

```
npm install @hippo-ai/sdk
```

## Quick start

```typescript
import { HippoClient } from "@hippo-ai/sdk";

const hippo = new HippoClient({
  baseUrl: "http://localhost:3000", // or set HIPPO_URL
  apiKey: "sk-...",                 // or set HIPPO_API_KEY
});

// Store a fact
await hippo.remember({ statement: "Alice likes cats" });

// Store many facts
await hippo.rememberBatch({
  statements: ["Bob knows Alice", "Bob lives in Portland"],
});

// Retrieve context
const ctx = await hippo.context({ query: "Alice", limit: 10 });
console.log(ctx.nodes, ctx.edges);

// Ask a question
const { answer } = await hippo.ask({ question: "What does Alice like?" });
console.log(answer);

// Health check (no auth required)
const health = await hippo.health();
```

## Admin operations

These endpoints require an admin API key.

```typescript
// Create a user (returns their first API key)
const { api_key } = await hippo.createUser({
  user_id: "agent-1",
  display_name: "Agent One",
});

// List users
const { users } = await hippo.listUsers();

// Manage API keys
await hippo.createKey("agent-1", { label: "secondary" });
const { keys } = await hippo.listKeys("agent-1");
await hippo.deleteKey("agent-1", "secondary");

// Remove a user
await hippo.deleteUser("agent-1");
```

## Error handling

```typescript
import {
  HippoError,
  AuthenticationError,
  ForbiddenError,
  RateLimitError,
} from "@hippo-ai/sdk";

try {
  await hippo.ask({ question: "hello" });
} catch (err) {
  if (err instanceof RateLimitError) {
    console.log("Retry after", err.retryAfter, "seconds");
  } else if (err instanceof AuthenticationError) {
    console.log("Bad credentials");
  } else if (err instanceof HippoError) {
    console.log("API error", err.status, err.message);
  }
}
```
