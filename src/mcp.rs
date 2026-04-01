use serde_json::{Map, Value};

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

// -- Helpers ------------------------------------------------------------------

/// Extract a required string argument, returning Err if missing.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required argument '{key}'"))
}

/// Build a JSON object from only the present (non-null) arguments.
fn build_body(pairs: &[(&str, Option<&Value>)]) -> Value {
    let mut map = Map::new();
    for &(key, val) in pairs {
        if let Some(v) = val {
            if !v.is_null() {
                map.insert(key.to_string(), v.clone());
            }
        }
    }
    Value::Object(map)
}

/// Send a POST request and return the response body, checking for HTTP errors.
fn post(
    client: &reqwest::blocking::Client,
    url: &str,
    body: &Value,
) -> Result<String, String> {
    let resp = client
        .post(url)
        .json(body)
        .send()
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }
    Ok(text)
}

/// Percent-encode a string for use as a URL path segment.
fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            // Unreserved characters (RFC 3986 §2.3)
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

// -- Tool call handlers -------------------------------------------------------

fn call_remember(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    require_str(args, "statement")?;
    let body = build_body(&[
        ("statement", args.get("statement")),
        ("source_agent", args.get("source_agent")),
    ]);
    post(client, &format!("{base_url}/remember"), &body)
}

fn call_recall(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    require_str(args, "query")?;
    let body = build_body(&[
        ("query", args.get("query")),
        ("limit", args.get("limit")),
        ("max_hops", args.get("max_hops")),
    ]);
    post(client, &format!("{base_url}/context"), &body)
}

fn call_reflect(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let body = build_body(&[
        ("about", args.get("about")),
    ]);
    post(client, &format!("{base_url}/reflect"), &body)
}

fn call_timeline(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    let entity = require_str(args, "entity")?;
    let encoded = encode_path_segment(entity);

    let resp = client
        .get(format!("{base_url}/timeline/{encoded}"))
        .send()
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }
    Ok(text)
}

fn call_recall_at(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Value,
) -> Result<String, String> {
    require_str(args, "query")?;
    require_str(args, "at")?;
    let body = build_body(&[
        ("query", args.get("query")),
        ("at", args.get("at")),
        ("limit", args.get("limit")),
    ]);
    post(client, &format!("{base_url}/context/temporal"), &body)
}
