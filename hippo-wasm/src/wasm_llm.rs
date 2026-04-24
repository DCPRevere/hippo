use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use hippo::llm_service::LlmService;
use hippo::math::{clean_json, normalize};
use hippo::models::{
    ContextFact, EdgeClassification, EntityRow,
    ExtractedEntity, GraphContext, OperationsResult, EMBEDDING_DIM,
};

pub struct WasmLlmClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    embedding_model: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    #[serde(rename = "max_completion_tokens")]
    max_tokens: u32,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageContent,
}

#[derive(Deserialize)]
struct ChatMessageContent {
    content: Option<String>,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl WasmLlmClient {
    pub fn new(
        api_key: String,
        base_url: String,
        model: String,
        embedding_model: String,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            base_url,
            model,
            embedding_model,
        }
    }

    async fn call(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let req = ChatRequest {
            model: self.model.clone(),
            max_tokens,
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user.to_string(),
                },
            ],
        };

        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&req)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error ({}): {}", status, body);
        }

        let r: ChatResponse = resp.json().await.context("failed to parse OpenAI response")?;
        r.choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .context("empty OpenAI response")
    }

    async fn embed_openai(&self, text: &str) -> Result<Vec<f32>> {
        let req = EmbeddingRequest {
            model: self.embedding_model.clone(),
            input: text.to_string(),
        };

        let resp = self
            .http
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&req)
            .send()
            .await
            .context("failed to call OpenAI embedding API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI embedding error ({}): {}", status, body);
        }

        let r: EmbeddingResponse = resp.json().await.context("failed to parse embedding response")?;
        let emb = r
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .context("empty embedding response")?;

        let mut v = emb;
        v.resize(EMBEDDING_DIM, 0.0);
        Ok(normalize(v))
    }
}

#[async_trait(?Send)]
impl LlmService for WasmLlmClient {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_openai(text).await
    }

    async fn extract_operations(
        &self,
        statement: &str,
        context: &GraphContext,
    ) -> Result<OperationsResult> {
        let subgraph_json = context.to_json();
        let system = r#"You are a knowledge graph mutation planner. Given a subgraph and a statement, determine the graph operations needed.

Rules:
- Reference existing entities by their node id from the subgraph.
- For new entities, emit create_node with a short "ref" (e.g. "n1"), then use that ref in edges.
- Generate BOTH directions for asymmetric relations: PARENT_OF needs a matching CHILD_OF.
- Generate only ONE direction for symmetric relations: MARRIED_TO, SIBLING_OF, KNOWS.
- Properties (surname, date_of_birth, nationality) go in create_node properties or update_node set, NOT as edges.
- When you learn an entity's full name (e.g. "James" becomes "James Taylor"), use update_node with "name" in the set field to rename it. Do NOT create SURNAME edges — update the name directly.
- The node with a "user_id" property is the person saying "I"/"me"/"my". Use its id for first-person references. If no such node exists in the subgraph, create one with the user's actual name (or "Me" if unknown) and set the property "user_id" to the value from the subgraph context.
- Confidence: 0.9+ for explicitly stated facts, 0.7-0.9 for inferred facts.
- Resolve pronouns ("they", "he", "she") to the correct entities from context.
- "Widower" means spouse has died — model as MARRIED_TO edge (DECEASED on the spouse captures the temporal aspect).

You MUST use EXACTLY this JSON schema. The discriminator field is "op" (NOT "operation").

create_node: {"op": "create_node", "ref": "n1", "name": "Alice", "type": "person", "properties": {}}
update_node: {"op": "update_node", "id": "<existing-node-id>", "set": {"name": "New Name"}}
create_edge: {"op": "create_edge", "from": "n1", "to": "n2", "relation": "KNOWS", "fact": "Alice knows Bob", "confidence": 0.9}
invalidate_edge: {"op": "invalidate_edge", "edge_id": 123, "fact": "old fact text", "reason": "superseded"}

"type" must be one of: person, place, organization, event, concept, content, unknown.
"relation" must be UPPER_SNAKE_CASE (e.g. PARENT_OF, WORKS_AT, MARRIED_TO).

Respond with ONLY valid JSON: {"operations": [...]}"#;

        let user = format!("Subgraph:\n{subgraph_json}\n\nNew statement: \"{statement}\"");
        let text = self.call(system, &user, 4096).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse operations result: {text}"))
    }

    async fn revise_operations(
        &self,
        original_ops: &OperationsResult,
        additional_context: &GraphContext,
    ) -> Result<OperationsResult> {
        let subgraph_json = additional_context.to_json();
        let ops_json = serde_json::to_string_pretty(original_ops)?;
        let system = "You are a knowledge graph mutation planner. You previously planned operations \
            but now have additional graph context. Revise the operations if needed — for example, \
            convert create_node to update_node if you now see the entity already exists, or add \
            new edges based on the additional context. \
            Use the same JSON schema: the discriminator field is \"op\" (not \"operation\"). \
            Return ONLY valid JSON with no markdown, no explanation: {\"operations\": [...]}";
        let user = format!(
            "Original planned operations:\n{ops_json}\n\nAdditional subgraph context discovered:\n{subgraph_json}\n\n\
            Revise the operations. Return the COMPLETE final operations list (not just changes)."
        );
        let text = self.call(system, &user, 4096).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse revised operations: {text}"))
    }

    async fn resolve_entities(
        &self,
        extracted: &ExtractedEntity,
        candidate: &EntityRow,
        candidate_facts: &[String],
    ) -> Result<(bool, f32)> {
        let facts_str = candidate_facts.join("; ");
        let system = "You are an entity resolution engine. Determine if two entity references refer to the same real-world entity. Respond with JSON: {\"same_entity\": true/false, \"confidence\": 0.0-1.0}";
        let user = format!(
            "New entity: \"{}\" (type: {})\nExisting entity: \"{}\" (type: {})\nExisting facts: {facts_str}",
            extracted.name, extracted.entity_type, candidate.name, candidate.entity_type
        );
        let text = self.call(system, &user, 256).await?;
        let text = clean_json(&text);
        let v: serde_json::Value = serde_json::from_str(text)?;
        let same = v["same_entity"].as_bool().unwrap_or(false);
        let conf = v["confidence"].as_f64().unwrap_or(0.5) as f32;
        Ok((same, conf))
    }

    async fn resolve_entities_batch(
        &self,
        pairs: &[(String, String, String, String, Vec<String>)],
    ) -> Result<Vec<(usize, bool, f32)>> {
        let mut results = Vec::new();
        for (i, (new_name, new_type, existing_name, existing_type, facts)) in pairs.iter().enumerate() {
            let facts_str = facts.join("; ");
            let system = "You are an entity resolution engine. Respond with JSON: {\"same_entity\": true/false, \"confidence\": 0.0-1.0}";
            let user = format!(
                "New: \"{new_name}\" ({new_type}), Existing: \"{existing_name}\" ({existing_type}), Facts: {facts_str}"
            );
            let text = self.call(system, &user, 256).await?;
            let text = clean_json(&text);
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let same = v["same_entity"].as_bool().unwrap_or(false);
            let conf = v["confidence"].as_f64().unwrap_or(0.5) as f32;
            results.push((i, same, conf));
        }
        Ok(results)
    }

    async fn classify_edge(
        &self,
        existing_fact: &str,
        new_fact: &str,
        relation_type: &str,
    ) -> Result<(EdgeClassification, f32)> {
        let system = "You are a fact classifier. Given two facts with the same relation type, classify as: \
            duplicate (same info), related (same topic but different info), contradiction (conflict), or unrelated. \
            Respond with JSON: {\"classification\": \"...\", \"confidence\": 0.0-1.0}";
        let user = format!(
            "Relation: {relation_type}\nExisting: \"{existing_fact}\"\nNew: \"{new_fact}\""
        );
        let text = self.call(system, &user, 256).await?;
        let text = clean_json(&text);
        let v: serde_json::Value = serde_json::from_str(text)?;
        let class = match v["classification"].as_str().unwrap_or("unrelated") {
            "duplicate" => EdgeClassification::Duplicate,
            "related" => EdgeClassification::Related,
            "contradiction" => EdgeClassification::Contradiction,
            _ => EdgeClassification::Unrelated,
        };
        let conf = v["confidence"].as_f64().unwrap_or(0.5) as f32;
        Ok((class, conf))
    }

    async fn discover_link(
        &self,
        a: &EntityRow,
        b: &EntityRow,
        a_facts: &[String],
        b_facts: &[String],
    ) -> Result<Option<(String, String, f32)>> {
        let system = "Given two entities and their known facts, determine if there is a plausible \
            relationship between them. Respond with JSON: {\"has_link\": true/false, \"relation\": \"...\", \"fact\": \"...\", \"confidence\": 0.0-1.0}";
        let user = format!(
            "Entity A: \"{}\" ({})\nFacts: {}\n\nEntity B: \"{}\" ({})\nFacts: {}",
            a.name, a.entity_type, a_facts.join("; "),
            b.name, b.entity_type, b_facts.join("; ")
        );
        let text = self.call(system, &user, 512).await?;
        let text = clean_json(&text);
        let v: serde_json::Value = serde_json::from_str(text)?;
        if v["has_link"].as_bool().unwrap_or(false) {
            let relation = v["relation"].as_str().unwrap_or("RELATED_TO").to_string();
            let fact = v["fact"].as_str().unwrap_or("").to_string();
            let conf = v["confidence"].as_f64().unwrap_or(0.5) as f32;
            Ok(Some((relation, fact, conf)))
        } else {
            Ok(None)
        }
    }

    async fn synthesise_answer(
        &self,
        question: &str,
        facts: &[ContextFact],
        user_display_name: Option<&str>,
    ) -> Result<String> {
        let facts_text: String = facts
            .iter()
            .map(|f| format!("- {} (confidence: {:.0}%)", f.fact, f.confidence * 100.0))
            .collect::<Vec<_>>()
            .join("\n");
        let name_hint = user_display_name
            .map(|n| format!(" The user's name is {n}."))
            .unwrap_or_default();
        let system = format!(
            "You are a helpful assistant that answers questions based on the provided knowledge graph facts. \
            Be concise and direct. If the facts don't contain enough information, say so.{name_hint}"
        );
        let user = format!("Facts:\n{facts_text}\n\nQuestion: {question}");
        self.call(&system, &user, 1024).await
    }

    async fn identify_missing_context(
        &self,
        question: &str,
        facts: &[ContextFact],
    ) -> Result<Vec<String>> {
        if facts.is_empty() {
            return Ok(vec![]);
        }

        let system = "You are a knowledge graph assistant. Given a question and a set of known facts, \
            determine if additional context about specific entities is needed to answer the question. \
            If the facts are sufficient, return an empty array. \
            If not, return the names of entities you need more facts about — specifically entities \
            that appear in the facts but whose relationships to each other are unclear. \
            Return ONLY valid JSON with no markdown.";

        let facts_block = facts.iter()
            .map(|f| format!("- {} ({}→{})", f.fact, f.subject, f.object))
            .collect::<Vec<_>>()
            .join("\n");

        let user = format!(
            "Question: {question}\n\nKnown facts:\n{facts_block}\n\n\
            Return JSON: {{\"entities\": [\"EntityName\", ...]}}\n\
            Return {{\"entities\": []}} if the facts are sufficient to answer."
        );

        let text = self.call(system, &user, 256).await?;
        let text = clean_json(&text);
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let entities = v["entities"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|e| e.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Ok(entities)
    }

    async fn find_missing_inferences(
        &self,
        _entity_name: &str,
        _entity_facts: &[String],
        _neighbor_facts: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String, String, f32)>> {
        Ok(vec![])
    }

    async fn generate_gap_questions(
        &self,
        _entity_name: &str,
        _known_facts: &[String],
        _gap_types: &[String],
    ) -> Result<Vec<String>> {
        Ok(vec![])
    }
}
