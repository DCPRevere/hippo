use serde_json::Value;

pub fn tool_definitions() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "remember",
            "description": "Store a new fact or statement into the agent's knowledge graph. Extracts entities and relationships automatically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "statement": {
                        "type": "string",
                        "description": "A natural language statement to remember"
                    },
                    "source_agent": {
                        "type": "string",
                        "description": "Identifier for the agent storing this fact (optional)"
                    }
                },
                "required": ["statement"]
            }
        }),
        serde_json::json!({
            "name": "recall",
            "description": "Retrieve relevant facts from the knowledge graph for a given query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for in memory"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of facts to return (default 10)",
                        "default": 10
                    },
                    "max_hops": {
                        "type": "integer",
                        "description": "Graph traversal depth (1-3, default 2)",
                        "default": 2
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "reflect",
            "description": "Introspect on what the agent knows about an entity, identify knowledge gaps, and get suggested questions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "about": {
                        "type": "string",
                        "description": "Entity name to reflect on (leave empty for global stats)"
                    }
                },
                "required": []
            }
        }),
        serde_json::json!({
            "name": "timeline",
            "description": "Get the chronological fact history for a specific entity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": {
                        "type": "string",
                        "description": "Entity name to get history for"
                    }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "recall_at",
            "description": "Query the knowledge graph as it existed at a specific point in time.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for"
                    },
                    "at": {
                        "type": "string",
                        "description": "ISO 8601 timestamp (e.g. 2024-01-15T00:00:00Z)"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 10
                    }
                },
                "required": ["query", "at"]
            }
        }),
    ]
}

pub fn handle_initialize(id: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "hippo-memory",
                "version": "0.1.0"
            }
        }
    })
}

pub fn handle_tools_list(id: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": tool_definitions()
        }
    })
}

pub fn handle_tool_call(
    id: Value,
    params: Value,
    client: &reqwest::blocking::Client,
    base_url: &str,
) -> Value {
    let tool_name = params["name"].as_str().unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    let result = match tool_name {
        "remember" => call_remember(client, base_url, &arguments),
        "recall" => call_recall(client, base_url, &arguments),
        "reflect" => call_reflect(client, base_url, &arguments),
        "timeline" => call_timeline(client, base_url, &arguments),
        "recall_at" => call_recall_at(client, base_url, &arguments),
        _ => Err(format!("Unknown tool: {tool_name}")),
    };

    match result {
        Ok(text) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": text }]
            }
        }),
        Err(e) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": format!("Error: {e}") }],
                "isError": true
            }
        }),
    }
}

pub fn make_error(id: Value, code: i64, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn call_remember(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let body = serde_json::json!({
        "statement": args["statement"],
        "source_agent": args.get("source_agent")
    });
    let resp = client
        .post(format!("{base_url}/remember"))
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    resp.text().map_err(|e| e.to_string())
}

fn call_recall(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let body = serde_json::json!({
        "query": args["query"],
        "limit": args.get("limit"),
        "max_hops": args.get("max_hops")
    });
    let resp = client
        .post(format!("{base_url}/context"))
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    resp.text().map_err(|e| e.to_string())
}

fn call_reflect(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let body = serde_json::json!({
        "about": args.get("about")
    });
    let resp = client
        .post(format!("{base_url}/reflect"))
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    resp.text().map_err(|e| e.to_string())
}

fn call_timeline(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let entity = args["entity"]
        .as_str()
        .ok_or_else(|| "missing 'entity' argument".to_string())?;
    let encoded = entity.replace('%', "%25").replace('/', "%2F").replace(' ', "%20");
    let resp = client
        .get(format!("{base_url}/timeline/{encoded}"))
        .send()
        .map_err(|e| e.to_string())?;
    resp.text().map_err(|e| e.to_string())
}

fn call_recall_at(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let body = serde_json::json!({
        "query": args["query"],
        "at": args["at"],
        "limit": args.get("limit")
    });
    let resp = client
        .post(format!("{base_url}/context/temporal"))
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    resp.text().map_err(|e| e.to_string())
}
