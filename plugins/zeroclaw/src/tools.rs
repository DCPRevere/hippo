use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::client::HippoClient;

// ---------------------------------------------------------------------------
// ZeroClaw Tool trait (local mirror -- keep in sync with your ZeroClaw version)
// ---------------------------------------------------------------------------

/// Result returned by a tool execution.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// ZeroClaw Tool trait.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;
}

// ---------------------------------------------------------------------------
// hippo_remember
// ---------------------------------------------------------------------------

/// Tool that stores a statement in the Hippo knowledge graph.
pub struct HippoRememberTool {
    client: HippoClient,
}

impl HippoRememberTool {
    pub fn new(client: HippoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for HippoRememberTool {
    fn name(&self) -> &str {
        "hippo_remember"
    }

    fn description(&self) -> &str {
        "Store a fact or statement in the Hippo knowledge graph"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "statement": {
                    "type": "string",
                    "description": "The fact or statement to remember"
                }
            },
            "required": ["statement"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let statement = args
            .get("statement")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: statement"))?;

        match self.client.remember(statement, "zeroclaw").await {
            Ok(resp) => Ok(ToolResult {
                success: true,
                output: format!(
                    "Stored. entities_created={}, facts_written={}, contradictions_invalidated={}",
                    resp.entities_created, resp.facts_written, resp.contradictions_invalidated
                ),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// hippo_recall
// ---------------------------------------------------------------------------

/// Tool that searches the Hippo knowledge graph for relevant facts.
pub struct HippoRecallTool {
    client: HippoClient,
}

impl HippoRecallTool {
    pub fn new(client: HippoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for HippoRecallTool {
    fn name(&self) -> &str {
        "hippo_recall"
    }

    fn description(&self) -> &str {
        "Search the Hippo knowledge graph for relevant facts"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 10)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        match self.client.context(query, limit).await {
            Ok(resp) => {
                if resp.facts.is_empty() {
                    return Ok(ToolResult {
                        success: true,
                        output: "No matching facts found.".to_string(),
                        error: None,
                    });
                }
                let lines: Vec<String> = resp
                    .facts
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        format!(
                            "{}. {} (confidence={:.2}, salience={})",
                            i + 1,
                            f.fact,
                            f.confidence,
                            f.salience,
                        )
                    })
                    .collect();
                Ok(ToolResult {
                    success: true,
                    output: lines.join("\n"),
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// hippo_ask
// ---------------------------------------------------------------------------

/// Tool that asks a natural-language question answered by the Hippo knowledge graph.
pub struct HippoAskTool {
    client: HippoClient,
}

impl HippoAskTool {
    pub fn new(client: HippoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for HippoAskTool {
    fn name(&self) -> &str {
        "hippo_ask"
    }

    fn description(&self) -> &str {
        "Ask a question answered by the Hippo knowledge graph"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask"
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: question"))?;

        match self.client.ask(question).await {
            Ok(resp) => Ok(ToolResult {
                success: true,
                output: resp.answer,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            }),
        }
    }
}
