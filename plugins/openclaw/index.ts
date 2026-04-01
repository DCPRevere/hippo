/**
 * @hippo-ai/openclaw-plugin
 *
 * OpenClaw memory plugin backed by Hippo. Provides three tools
 * (hippo_remember, hippo_recall, hippo_ask) and optional auto-recall
 * and auto-capture lifecycle hooks.
 */

interface OpenClawPluginApi {
  pluginConfig?: Record<string, unknown>;
  logger: { info(msg: string): void; warn(msg: string): void };
  registerTool(tool: ToolDefinition): void;
  registerService(service: ServiceDefinition): void;
  on(event: string, handler: (event: unknown) => Promise<unknown | void>): void;
}

interface ToolDefinition {
  name: string;
  label: string;
  description: string;
  parameters: Record<string, unknown>;
  execute(toolCallId: string, params: Record<string, unknown>): Promise<ToolResult>;
}

interface ToolResult {
  content: Array<{ type: string; text: string }>;
}

interface ServiceDefinition {
  id: string;
  start(): Promise<void>;
  stop(): Promise<void>;
}

interface OpenClawPluginDefinition {
  id: string;
  register(api: OpenClawPluginApi): void;
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

interface HippoConfig {
  baseUrl: string;
  apiKey: string;
  graphName: string | undefined;
  autoCapture: boolean;
  autoRecall: boolean;
  maxRecallFacts: number;
}

function readConfig(api: OpenClawPluginApi): HippoConfig {
  const pc = api.pluginConfig ?? {};
  return {
    baseUrl:
      (pc.baseUrl as string) ||
      process.env.HIPPO_URL ||
      "http://localhost:21693",
    apiKey: (pc.apiKey as string) || process.env.HIPPO_API_KEY || "",
    graphName: (pc.graphName as string) || undefined,
    autoCapture: pc.autoCapture !== false,
    autoRecall: pc.autoRecall !== false,
    maxRecallFacts: (pc.maxRecallFacts as number) || 10,
  };
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

function createHippoClient(config: HippoConfig) {
  return async function hippoFetch(
    path: string,
    opts: { method?: string; body?: unknown } = {},
  ): Promise<any> {
    const url = `${config.baseUrl}${path}`;
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (config.apiKey) {
      headers["Authorization"] = `Bearer ${config.apiKey}`;
    }

    const resp = await fetch(url, {
      method: opts.method || "POST",
      headers,
      body: opts.body ? JSON.stringify(opts.body) : undefined,
    });

    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`Hippo API error ${resp.status}: ${text}`);
    }

    return resp.json();
  };
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

function registerTools(
  api: OpenClawPluginApi,
  config: HippoConfig,
  hippoFetch: ReturnType<typeof createHippoClient>,
): void {
  api.registerTool({
    name: "hippo_remember",
    label: "Remember in Hippo",
    description:
      "Store a fact or statement in the Hippo knowledge graph for long-term memory",
    parameters: {
      type: "object",
      properties: {
        statement: {
          type: "string",
          description: "The fact or statement to remember",
        },
        source_agent: {
          type: "string",
          description: "Source agent identifier",
        },
      },
      required: ["statement"],
    },
    async execute(_toolCallId: string, params: Record<string, unknown>) {
      const body: Record<string, unknown> = {
        statement: params.statement,
      };
      if (params.source_agent) body.source_agent = params.source_agent;
      if (config.graphName) body.graph = config.graphName;

      const result = await hippoFetch("/remember", { body });
      return {
        content: [
          {
            type: "text",
            text: `Stored: ${result.entities_created} entities, ${result.facts_written} facts`,
          },
        ],
      };
    },
  });

  api.registerTool({
    name: "hippo_recall",
    label: "Recall from Hippo",
    description:
      "Search the Hippo knowledge graph for facts relevant to a query",
    parameters: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "The query to search for",
        },
        limit: {
          type: "number",
          description: "Maximum number of facts to return",
        },
      },
      required: ["query"],
    },
    async execute(_toolCallId: string, params: Record<string, unknown>) {
      const body: Record<string, unknown> = {
        query: params.query,
        limit: (params.limit as number) || config.maxRecallFacts,
      };
      if (config.graphName) body.graph = config.graphName;

      const ctx = await hippoFetch("/context", { body });
      const edges: Array<{ fact: string; confidence: number }> =
        ctx.edges || [];

      if (!edges.length) {
        return {
          content: [{ type: "text", text: "No relevant facts found." }],
        };
      }

      const lines = edges.map(
        (e) =>
          `- ${e.fact} (confidence: ${Math.round(e.confidence * 100)}%)`,
      );
      return {
        content: [{ type: "text", text: lines.join("\n") }],
      };
    },
  });

  api.registerTool({
    name: "hippo_ask",
    label: "Ask Hippo",
    description:
      "Ask a natural-language question answered by the Hippo knowledge graph",
    parameters: {
      type: "object",
      properties: {
        question: {
          type: "string",
          description: "The question to ask",
        },
      },
      required: ["question"],
    },
    async execute(_toolCallId: string, params: Record<string, unknown>) {
      const body: Record<string, unknown> = {
        question: params.question,
      };
      if (config.graphName) body.graph = config.graphName;

      const result = await hippoFetch("/ask", { body });
      return {
        content: [{ type: "text", text: result.answer || String(result) }],
      };
    },
  });
}

// ---------------------------------------------------------------------------
// Lifecycle hooks
// ---------------------------------------------------------------------------

function registerHooks(
  api: OpenClawPluginApi,
  config: HippoConfig,
  hippoFetch: ReturnType<typeof createHippoClient>,
): void {
  if (config.autoRecall) {
    api.on("before_agent_start", async (event: unknown) => {
      const prompt = (event as Record<string, unknown>).prompt as string;
      if (!prompt?.trim()) return;

      try {
        const body: Record<string, unknown> = {
          query: prompt,
          limit: config.maxRecallFacts,
        };
        if (config.graphName) body.graph = config.graphName;

        const ctx = await hippoFetch("/context", { body });
        const edges: Array<{ fact: string; confidence: number }> =
          ctx.edges || [];
        if (!edges.length) return;

        const facts = edges
          .map(
            (e) =>
              `- ${e.fact} (confidence: ${Math.round(e.confidence * 100)}%)`,
          )
          .join("\n");

        return {
          prependContext: `<hippo-knowledge>\nRelevant facts from long-term memory:\n${facts}\n</hippo-knowledge>`,
        };
      } catch (err) {
        api.logger.warn(`hippo auto-recall failed: ${err}`);
      }
    });
  }

  if (config.autoCapture) {
    api.on("agent_end", async (event: unknown) => {
      const ev = event as Record<string, unknown>;
      if (!ev.success || !Array.isArray(ev.messages)) return;

      try {
        const messages = ev.messages as Array<{
          role: string;
          content: unknown;
        }>;

        const lastUser = [...messages]
          .reverse()
          .find((m) => m.role === "user");
        const lastAssistant = [...messages]
          .reverse()
          .find((m) => m.role === "assistant");

        if (!lastUser || !lastAssistant) return;

        const userText =
          typeof lastUser.content === "string"
            ? lastUser.content
            : JSON.stringify(lastUser.content);
        const assistantText =
          typeof lastAssistant.content === "string"
            ? lastAssistant.content
            : JSON.stringify(lastAssistant.content);

        const statement = `User asked: "${userText.slice(0, 500)}". The answer was: "${assistantText.slice(0, 500)}"`;
        const body: Record<string, unknown> = {
          statement,
          source_agent: "openclaw",
        };
        if (config.graphName) body.graph = config.graphName;

        await hippoFetch("/remember", { body });
      } catch (err) {
        api.logger.warn(`hippo auto-capture failed: ${err}`);
      }
    });
  }
}

// ---------------------------------------------------------------------------
// Service lifecycle
// ---------------------------------------------------------------------------

function registerService(
  api: OpenClawPluginApi,
  hippoFetch: ReturnType<typeof createHippoClient>,
): void {
  api.registerService({
    id: "openclaw-hippo",
    async start() {
      try {
        await hippoFetch("/health", { method: "GET" });
        api.logger.info("hippo: connected");
      } catch (err) {
        api.logger.warn(`hippo: connection failed: ${err}`);
      }
    },
    async stop() {
      api.logger.info("hippo: stopped");
    },
  });
}

// ---------------------------------------------------------------------------
// Plugin definition
// ---------------------------------------------------------------------------

const plugin: OpenClawPluginDefinition = {
  id: "openclaw-hippo",

  register(api: OpenClawPluginApi) {
    const config = readConfig(api);
    const hippoFetch = createHippoClient(config);

    registerTools(api, config, hippoFetch);
    registerHooks(api, config, hippoFetch);
    registerService(api, hippoFetch);
  },
};

export default plugin;
