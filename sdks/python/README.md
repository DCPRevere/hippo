# hippo-sdk

Python SDK for the [Hippo](https://github.com/dcprevere/hippo) natural-language knowledge graph API.

## Install

```bash
pip install hippo-sdk
```

## Quick start

```python
from hippo import HippoClient

client = HippoClient(base_url="http://localhost:3000", api_key="your-key")

# Store a fact
result = client.remember("Alice works at Acme Corp")
print(result.facts_written)

# Query context
ctx = client.context("Tell me about Alice")
print(ctx.nodes)

# Ask a question
answer = client.ask("Where does Alice work?")
print(answer.answer)
```

## Configuration

Both `HippoClient` and `AsyncHippoClient` accept `base_url` and `api_key` as constructor arguments. If omitted, they fall back to the `HIPPO_URL` and `HIPPO_API_KEY` environment variables.

```bash
export HIPPO_URL=http://localhost:3000
export HIPPO_API_KEY=your-key
```

```python
client = HippoClient()  # uses env vars
```

## Async usage

```python
from hippo import AsyncHippoClient

async with AsyncHippoClient() as client:
    result = await client.remember("Bob likes Python")
    answer = await client.ask("What does Bob like?")
```

## Batch ingestion

```python
result = client.remember_batch(
    ["Alice is 30 years old", "Alice lives in Portland"],
    parallel=True,
)
print(f"{result.succeeded}/{result.total} succeeded")
```

## Admin operations

These require an admin API key.

```python
# Create a user
user = client.create_user("alice", "Alice A.", role="reader", graphs=["default"])
print(user.api_key)

# List users
users = client.list_users()

# Manage API keys
key = client.create_key("alice", "laptop")
keys = client.list_keys("alice")
client.delete_key("alice", "laptop")

# Delete a user
client.delete_user("alice")
```

## Error handling

```python
from hippo import HippoError, AuthenticationError, ForbiddenError, RateLimitError

try:
    client.remember("fact")
except AuthenticationError:
    print("Invalid API key")
except ForbiddenError:
    print("Insufficient permissions")
except RateLimitError:
    print("Too many requests, back off")
except HippoError as e:
    print(f"HTTP {e.status_code}: {e.body}")
```
