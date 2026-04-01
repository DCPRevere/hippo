# @hippo-ai/openclaw-plugin

OpenClaw memory plugin backed by [Hippo](https://github.com/dcprevere/hippo). Gives OpenClaw agents persistent knowledge-graph memory -- they can remember facts, recall relevant context, and ask natural-language questions against the graph.

## Installation

```
openclaw plugins install @hippo-ai/openclaw-plugin
```

## Configuration

Add the plugin to your `openclaw.json`:

```json
{
  "plugins": {
    "@hippo-ai/openclaw-plugin": {
      "baseUrl": "http://localhost:21693",
      "apiKey": "your-api-key",
      "graphName": "my-project",
      "autoCapture": true,
      "autoRecall": true,
      "maxRecallFacts": 10
    }
  }
}
```

All fields are optional. Defaults are shown above (except `apiKey` and `graphName` which have no default).

## Environment Variables

| Variable | Description |
|---|---|
| `HIPPO_URL` | Hippo API base URL (fallback when `baseUrl` is not set in config) |
| `HIPPO_API_KEY` | Bearer token for Hippo API auth (fallback when `apiKey` is not set) |

## Tools

The plugin registers three tools that agents can call:

**hippo_remember** -- Store a fact or statement in the knowledge graph.
- `statement` (string, required): The fact to remember.
- `source_agent` (string, optional): Identifier for the source agent.

**hippo_recall** -- Search the knowledge graph for relevant facts.
- `query` (string, required): The search query.
- `limit` (number, optional): Max facts to return (default: 10).

**hippo_ask** -- Ask a natural-language question answered by the graph.
- `question` (string, required): The question to ask.

## Auto-recall

When `autoRecall` is enabled (default), the plugin hooks into `before_agent_start` and automatically queries Hippo with the user's prompt. Relevant facts are prepended to the agent's context inside a `<hippo-knowledge>` block.

## Auto-capture

When `autoCapture` is enabled (default), the plugin hooks into `agent_end` and stores a summary of the last user/assistant exchange in Hippo. This builds up the knowledge graph passively over time.

## License

MIT
