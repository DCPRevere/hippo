use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::config::LlmProvider;
use crate::fixtures::{self, FixtureStore, LlmFixture};
use crate::models::{
    EdgeClassification, EntityRow, ExtractionResult, ExtractedEntity, ExtractedFact, EMBEDDING_DIM,
};

/// Canonical relation pairs: (forward, inverse).
/// For symmetric relations, forward == inverse.
pub const RELATION_PAIRS: &[(&str, &str)] = &[
    ("PARENT_OF", "CHILD_OF"),
    ("CHILD_OF", "PARENT_OF"),
    ("MARRIED_TO", "MARRIED_TO"),
    ("SIBLING_OF", "SIBLING_OF"),
    ("WORKS_AT", "EMPLOYS"),
    ("EMPLOYS", "WORKS_AT"),
    ("OWNS", "OWNED_BY"),
    ("OWNED_BY", "OWNS"),
    ("LEADS", "LED_BY"),
    ("LED_BY", "LEADS"),
    ("KNOWS", "KNOWS"),
];

/// Returns the inverse relation type if one exists in the canonical pairs.
pub fn inverse_relation(relation_type: &str) -> Option<&'static str> {
    RELATION_PAIRS.iter()
        .find(|(fwd, _)| *fwd == relation_type)
        .map(|(_, inv)| *inv)
}

/// Returns true if the relation is symmetric (inverse equals itself).
pub fn is_symmetric(relation_type: &str) -> bool {
    RELATION_PAIRS.iter()
        .any(|(fwd, inv)| *fwd == relation_type && *fwd == *inv)
}

#[derive(Debug, Clone, PartialEq)]
pub enum FixtureMode {
    None,
    Record,
    Replay,
}

pub enum AnthropicAuth {
    ApiKey(String),
    OAuthToken(String),
}

pub struct LlmClient {
    http: reqwest::Client,
    auth: AnthropicAuth,
    model: String,
    ollama_url: String,
    fixture_mode: FixtureMode,
    fixture_store: Arc<RwLock<FixtureStore>>,
    fixture_path: PathBuf,
    mock_mode: bool,
    provider: LlmProvider,
    openai_api_key: Option<String>,
    openai_base_url: String,
    openai_model: String,
    openai_embedding_model: Option<String>,
    pub max_tokens: u32,
    extraction_prompt: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    stream: bool,
    system: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize)]
struct AnthropicToolChoice {
    #[serde(rename = "type")]
    choice_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize, Default, Clone)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

// OpenAI chat completions types

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
}

// OpenAI embedding types

#[derive(Serialize)]
struct OpenAIEmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
}

#[derive(Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

impl LlmClient {
    pub fn new(
        auth: AnthropicAuth,
        model: String,
        ollama_url: String,
        http: reqwest::Client,
        fixture_mode: FixtureMode,
        fixture_path: PathBuf,
        mock_mode: bool,
        max_tokens: u32,
        extraction_prompt: String,
    ) -> Self {
        let fixture_store = Arc::new(RwLock::new(FixtureStore::load(&fixture_path)));
        Self {
            http, auth, model, ollama_url, fixture_mode, fixture_store, fixture_path, mock_mode,
            provider: LlmProvider::Anthropic,
            openai_api_key: None,
            openai_base_url: "https://api.openai.com/v1".to_string(),
            openai_model: "gpt-4o-mini".to_string(),
            openai_embedding_model: None,
            max_tokens,
            extraction_prompt,
        }
    }

    pub fn with_openai(
        mut self,
        api_key: Option<String>,
        base_url: String,
        model: String,
        embedding_model: Option<String>,
    ) -> Self {
        self.provider = LlmProvider::OpenAI;
        self.openai_api_key = api_key;
        self.openai_base_url = base_url;
        self.openai_model = model;
        self.openai_embedding_model = embedding_model;
        self
    }

    pub async fn extract_entities_and_facts(&self, statement: &str) -> Result<ExtractionResult> {
        if self.mock_mode {
            return self.extract_entities_and_facts_mock(statement);
        }

        let system = "You are a knowledge extraction agent. Extract all entities, explicit facts, \
            and implied facts from the given statement. Return ONLY valid JSON with no markdown, \
            no explanation, no code fences.";

        let user = format!(
            r#"Extract from this statement: "{statement}"

Return JSON matching this exact schema:
{{
  "entities": [
    {{
      "name": "string",
      "entity_type": "string (person|place|event|concept|organization|content|unknown)",
      "resolved": true,
      "hint": null,
      "content": null
    }}
  ],
  "explicit_facts": [
    {{
      "subject": "string (entity name from entities list)",
      "relation_type": "string (e.g. ATTENDED, MARRIED_TO, WORKS_AT, KNOWS, OWNS, PRESCRIBED_BY, BILLS)",
      "object": "string (entity name from entities list)",
      "fact": "string (full natural language fact statement)",
      "confidence": 0.9
    }}
  ],
  "implied_facts": []
}}

BIDIRECTIONAL RELATION RULES — follow these strictly:

Asymmetric pairs (always generate BOTH directions):
  PARENT_OF ↔ CHILD_OF, WORKS_AT ↔ EMPLOYS, OWNS ↔ OWNED_BY, LEADS ↔ LED_BY

Symmetric relations (generate only ONE direction, never duplicate):
  MARRIED_TO, SIBLING_OF, KNOWS

For asymmetric relations: put the explicit fact in explicit_facts and its inverse in implied_facts.
Example: "Bob is father of Charlie"
  → explicit_facts: {{"subject":"Bob","relation_type":"PARENT_OF","object":"Charlie","fact":"Bob is the father of Charlie","confidence":0.95}}
  → implied_facts: {{"subject":"Charlie","relation_type":"CHILD_OF","object":"Bob","fact":"Charlie is a child of Bob","confidence":0.95}}

For symmetric relations like MARRIED_TO or SIBLING_OF, only produce one fact (do NOT add an inverse).

Include the narrator/speaker as "Me" in entities when first-person language is used.
For unknown entities (e.g. "John's spouse"), set resolved=false and provide a hint.
When a surname can be inferred from context (e.g. children of a named parent, maiden names), use the full name for the entity.
Attributes (surname, date_of_birth, nationality, etc.) ALWAYS go in entity_attributes, NEVER as facts in implied_facts or explicit_facts.
Use implied_facts ONLY for relationships between entities (e.g. SIBLING_OF, CHILD_OF, PARENT_OF).
For shell commands, quotations, code snippets, or other verbatim text, use entity_type="content" and put the full verbatim text in the "content" field."#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse extraction result — LLM returned: {text}"))
    }

    pub async fn extract_entities_and_facts_with_context(
        &self,
        statement: &str,
        context: &crate::models::GraphContext,
    ) -> Result<ExtractionResult> {
        if context.is_empty() {
            return self.extract_entities_and_facts(statement).await;
        }
        if self.mock_mode {
            return self.extract_entities_and_facts_mock(statement);
        }

        let context_block = context.to_json();

        let system = "You are a knowledge extraction agent. Extract all entities, explicit facts, \
            and implied facts from the given statement. Use the provided graph context to make \
            better inferences (e.g. inferring surnames, full names, relationships). \
            IMPORTANT: Reuse existing entity names from the graph where they refer to the same \
            real-world entity. If an entity in the statement is a more complete version of an \
            existing name (e.g. 'Bob Smith' for existing 'Bob'), use the more complete name \
            but note this in entity_updates. \
            Return ONLY valid JSON with no markdown, no explanation, no code fences.";

        let user = format!(
            r#"{context_block}
Extract from this statement: "{statement}"

CRITICAL — entity naming rules:
- If an entity from the statement matches an existing entity in the graph above, use the EXISTING name.
- If the statement reveals a more complete name for an existing entity (e.g. "Bob" should become "Bob Smith"), use the new full name in entities AND add an entity_update entry.
- Attributes like surname, date of birth, nationality are NOT relationships — put them in entity_attributes, not in facts.

Return JSON matching this exact schema:
{{
  "entities": [
    {{
      "name": "string",
      "entity_type": "string (person|place|event|concept|organization|content|unknown)",
      "resolved": true,
      "hint": null,
      "content": null
    }}
  ],
  "entity_updates": [
    {{
      "old_name": "string (existing name in graph)",
      "new_name": "string (more complete name)",
      "reason": "string"
    }}
  ],
  "entity_attributes": [
    {{
      "entity": "string (entity name)",
      "attribute": "string (e.g. surname, date_of_birth, nationality)",
      "value": "string",
      "confidence": 0.9
    }}
  ],
  "explicit_facts": [
    {{
      "subject": "string (entity name from entities list)",
      "relation_type": "string (e.g. ATTENDED, MARRIED_TO, WORKS_AT, KNOWS, OWNS, PRESCRIBED_BY, BILLS)",
      "object": "string (entity name from entities list)",
      "fact": "string (full natural language fact statement)",
      "confidence": 0.9
    }}
  ],
  "implied_facts": []
}}

BIDIRECTIONAL RELATION RULES — follow these strictly:

Asymmetric pairs (always generate BOTH directions):
  PARENT_OF ↔ CHILD_OF, WORKS_AT ↔ EMPLOYS, OWNS ↔ OWNED_BY, LEADS ↔ LED_BY

Symmetric relations (generate only ONE direction, never duplicate):
  MARRIED_TO, SIBLING_OF, KNOWS

For asymmetric relations: put the explicit fact in explicit_facts and its inverse in implied_facts.

For symmetric relations like MARRIED_TO or SIBLING_OF, only produce one fact (do NOT add an inverse).

Include the narrator/speaker as "Me" in entities when first-person language is used.
For unknown entities (e.g. "John's spouse"), set resolved=false and provide a hint.
When a surname can be inferred from context (e.g. children of a named parent, maiden names), use the full name for the entity.
Attributes (surname, date_of_birth, nationality, etc.) ALWAYS go in entity_attributes, NEVER as facts in implied_facts or explicit_facts.
Use implied_facts ONLY for relationships between entities (e.g. SIBLING_OF, CHILD_OF, PARENT_OF).
For shell commands, quotations, code snippets, or other verbatim text, use entity_type="content" and put the full verbatim text in the "content" field."#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse extraction result — LLM returned: {text}"))
    }

    pub async fn infer_additional_facts(
        &self,
        extracted_facts: &[ExtractedFact],
        entity_context: &[(String, Vec<String>)],
    ) -> Result<crate::models::EnrichmentResult> {
        if self.mock_mode || entity_context.is_empty() {
            return Ok(crate::models::EnrichmentResult {
                entity_attributes: vec![],
                facts: vec![],
            });
        }

        let mut facts_block = String::from("Newly extracted facts:\n");
        for f in extracted_facts {
            facts_block.push_str(&format!("- \"{}\" ({} -> {})\n", f.fact, f.subject, f.object));
        }

        let mut context_block = String::from("\nKnown context for resolved entities:\n");
        for (name, facts) in entity_context {
            if facts.is_empty() {
                context_block.push_str(&format!("- {name}: (no existing facts)\n"));
            } else {
                let facts_str = facts.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
                context_block.push_str(&format!("- {name}: {facts_str}\n"));
            }
        }

        let system = "You are a knowledge inference agent. Given newly extracted facts and \
            existing knowledge about entities, identify additional facts and entity attributes \
            that can be strongly inferred but were not explicitly extracted. \
            Return ONLY valid JSON with no markdown.";

        let user = format!(
            r#"{facts_block}{context_block}
Infer additional relationships and attributes that follow logically. Examples:
- If a person marries someone with a known surname, the spouse likely takes that surname
- Children of named parents inherit the family surname
- Siblings share parents

IMPORTANT:
- Attributes (surname, date_of_birth, nationality, etc.) go in entity_attributes, NOT in facts.
- Facts are ONLY for relationships between entities (PARENT_OF, CHILD_OF, SIBLING_OF, etc.).
- relation_type MUST be UPPER_SNAKE_CASE.
- Do NOT duplicate facts or attributes that already exist in the known context above.
- Subject and object in facts MUST be entity names, not bare attribute values.

Return JSON:
{{
  "entity_attributes": [
    {{"entity": "string", "attribute": "string", "value": "string", "confidence": 0.7}}
  ],
  "facts": [
    {{"subject": "string", "relation_type": "STRING_UPPER", "object": "string (entity name)", "fact": "string", "confidence": 0.7}}
  ]
}}

Only include items with confidence >= 0.7. Return empty arrays if nothing can be strongly inferred."#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse enrichment result — LLM returned: {text}"))
    }

    pub async fn resolve_entities(
        &self,
        extracted: &ExtractedEntity,
        candidate: &EntityRow,
        candidate_facts: &[String],
    ) -> Result<(bool, f32)> {
        if self.mock_mode {
            return Ok((false, 0.5));
        }

        let system = "You are an entity resolution agent. Decide whether two entity descriptions \
            refer to the same real-world entity. A person referred to by first name only and a \
            person with that first name plus a surname ARE the same entity if the context supports it \
            (e.g. family relationships, shared facts). \
            Respond with ONLY a JSON object. No explanation, no markdown, no text before or after the JSON.";

        let facts_block = if candidate_facts.is_empty() {
            String::new()
        } else {
            let facts_str = candidate_facts.iter().map(|f| format!("  - \"{f}\"")).collect::<Vec<_>>().join("\n");
            format!("\nKnown facts about Entity B:\n{facts_str}\n")
        };

        let user = format!(
            r#"Entity A (new): name="{}", type="{}", hint="{}"
Entity B (existing in graph): name="{}", type="{}"
{facts_block}
Are these the same real-world entity? Consider:
- "Bob Smith" and "Bob" are the same person if Bob's graph facts show family connections to other Smiths
- A full name is a refinement of a first-name-only entity, not a different person
- Use the known facts to determine if the context matches

Return: {{"same_entity": true/false, "confidence": 0.0-1.0}}"#,
            extracted.name,
            extracted.entity_type,
            extracted.hint.as_deref().unwrap_or(""),
            candidate.name,
            candidate.entity_type,
        );

        let text = self.call(system, &user, 256).await?;
        let text = clean_json(&text);
        let v: Value = serde_json::from_str(text).with_context(|| {
            format!("failed to parse entity resolution — LLM returned: {text}")
        })?;
        let same = v["same_entity"].as_bool().unwrap_or(false);
        let confidence = v["confidence"].as_f64().unwrap_or(0.0) as f32;
        Ok((same, confidence))
    }

    /// Batch entity resolution: given a list of (entity_a, entity_b, b_facts) pairs,
    /// determine which pairs are the same real-world entity in a single LLM call.
    pub async fn resolve_entities_batch(
        &self,
        pairs: &[(String, String, String, String, Vec<String>)], // (a_name, a_type, b_name, b_type, b_facts)
    ) -> Result<Vec<(usize, bool, f32)>> {
        if self.mock_mode || pairs.is_empty() {
            return Ok(pairs.iter().enumerate().map(|(i, _)| (i, false, 0.5)).collect());
        }

        let system = "You are an entity resolution agent. For each numbered pair, decide whether \
            the two entities refer to the same real-world entity. Consider name variants (e.g. \
            'Bob' vs 'Bob Smith'), shared facts, and context. \
            Return ONLY valid JSON with no markdown.";

        let mut pairs_block = String::new();
        for (i, (a_name, a_type, b_name, b_type, b_facts)) in pairs.iter().enumerate() {
            pairs_block.push_str(&format!("Pair {i}:\n"));
            pairs_block.push_str(&format!("  Entity A: name=\"{a_name}\", type=\"{a_type}\"\n"));
            pairs_block.push_str(&format!("  Entity B: name=\"{b_name}\", type=\"{b_type}\"\n"));
            if !b_facts.is_empty() {
                let facts_str = b_facts.iter().take(5).map(|f| format!("    - \"{f}\"")).collect::<Vec<_>>().join("\n");
                pairs_block.push_str(&format!("  Known facts about B:\n{facts_str}\n"));
            }
            pairs_block.push('\n');
        }

        let user = format!(
            r#"{pairs_block}
For each pair, determine if they are the same entity.
Return a JSON array:
[{{"index": 0, "same_entity": true, "confidence": 0.9}}, ...]"#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        let items: Vec<Value> = serde_json::from_str(text)
            .with_context(|| format!("failed to parse batch entity resolution — LLM returned: {text}"))?;

        let mut results = Vec::with_capacity(items.len());
        for item in items {
            let index = item["index"].as_u64().unwrap_or(0) as usize;
            let same = item["same_entity"].as_bool().unwrap_or(false);
            let confidence = item["confidence"].as_f64().unwrap_or(0.0) as f32;
            results.push((index, same, confidence));
        }

        Ok(results)
    }

    pub async fn classify_edge(
        &self,
        existing_fact: &str,
        new_fact: &str,
        relation_type: &str,
    ) -> Result<(EdgeClassification, f32)> {
        if self.mock_mode {
            return Ok((EdgeClassification::Unrelated, 0.5));
        }

        let system = "You are a fact classification agent. Decide the relationship between two \
            facts about the same entity pair. Return ONLY valid JSON with no markdown.";

        let user = format!(
            r#"Existing fact: "{existing_fact}"
New fact: "{new_fact}"
Relation type: "{relation_type}"

Classify:
- "duplicate": same meaning, no new information
- "contradiction": new fact conflicts with existing (existing should be invalidated)
- "related": related but both can coexist
- "unrelated": different aspects, both valid

Return: {{"classification": "contradiction", "confidence": 0.9}}"#
        );

        let text = self.call(system, &user, 256).await?;
        let text = clean_json(&text);
        let v: Value = serde_json::from_str(text)
            .with_context(|| format!("failed to parse edge classification — LLM returned: {text}"))?;
        let classification = match v["classification"].as_str().unwrap_or("unrelated") {
            "duplicate" => EdgeClassification::Duplicate,
            "contradiction" => EdgeClassification::Contradiction,
            "related" => EdgeClassification::Related,
            _ => EdgeClassification::Unrelated,
        };
        let confidence = v["confidence"].as_f64().unwrap_or(0.5) as f32;
        Ok((classification, confidence))
    }

    /// Classify multiple existing facts against a new fact in a single LLM call.
    /// Returns a vec of (edge_index, classification, confidence).
    pub async fn classify_edges_batch(
        &self,
        existing_facts: &[&str],
        new_fact: &str,
        relation_type: &str,
    ) -> Result<Vec<(usize, EdgeClassification, f32)>> {
        if self.mock_mode {
            return Ok(existing_facts.iter().enumerate()
                .map(|(i, _)| (i, EdgeClassification::Unrelated, 0.5))
                .collect());
        }

        let system = "You are a fact classification agent. Decide the relationship between each \
            existing fact and a new fact about the same entity pair. Return ONLY valid JSON with no markdown.";

        let numbered_facts: String = existing_facts
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{i}: \"{f}\""))
            .collect::<Vec<_>>()
            .join("\n");

        let user = format!(
            r#"New fact: "{new_fact}"
Relation type: "{relation_type}"

Existing facts:
{numbered_facts}

For each existing fact, classify its relationship to the new fact:
- "duplicate": same meaning, no new information
- "contradiction": new fact conflicts with existing (existing should be invalidated)
- "related": related but both can coexist
- "unrelated": different aspects, both valid

Return a JSON array:
[{{"index": 0, "classification": "unrelated", "confidence": 0.9}}, ...]"#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        let items: Vec<Value> = serde_json::from_str(text)
            .with_context(|| format!("failed to parse batch edge classification — LLM returned: {text}"))?;

        let mut results = Vec::with_capacity(items.len());
        for item in items {
            let index = item["index"].as_u64().unwrap_or(0) as usize;
            let classification = match item["classification"].as_str().unwrap_or("unrelated") {
                "duplicate" => EdgeClassification::Duplicate,
                "contradiction" => EdgeClassification::Contradiction,
                "related" => EdgeClassification::Related,
                _ => EdgeClassification::Unrelated,
            };
            let confidence = item["confidence"].as_f64().unwrap_or(0.5) as f32;
            results.push((index, classification, confidence));
        }

        Ok(results)
    }

    pub async fn discover_link(
        &self,
        a: &EntityRow,
        b: &EntityRow,
        a_facts: &[String],
        b_facts: &[String],
    ) -> Result<Option<(String, String, f32)>> {
        if self.mock_mode {
            return Ok(None);
        }

        let system = "You are a knowledge graph agent. Decide whether two entities should have a \
            direct relationship based on their known facts. Only create a link if clearly warranted \
            by existing facts — do not invent new information. Return ONLY valid JSON with no markdown.";

        let user = format!(
            r#"Entity A: name="{}", type="{}"
Known facts about A: {}

Entity B: name="{}", type="{}"
Known facts about B: {}

Should these entities have a direct relationship edge?
Return: {{"create_edge": false, "relation_type": null, "fact": null, "confidence": 0.0}}"#,
            a.name,
            a.entity_type,
            a_facts.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n"),
            b.name,
            b.entity_type,
            b_facts.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n"),
        );

        let text = self.call(system, &user, 256).await?;
        let text = clean_json(&text);
        let v: Value = serde_json::from_str(text)
            .with_context(|| format!("failed to parse link discovery — LLM returned: {text}"))?;
        if v["create_edge"].as_bool().unwrap_or(false) {
            let rel_type = v["relation_type"].as_str().unwrap_or("RELATED").to_string();
            let fact = v["fact"].as_str().unwrap_or("").to_string();
            let confidence = v["confidence"].as_f64().unwrap_or(0.5) as f32;
            Ok(Some((rel_type, fact, confidence)))
        } else {
            Ok(None)
        }
    }

    pub async fn find_missing_inferences(
        &self,
        entity_name: &str,
        entity_facts: &[String],
        neighbor_facts: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String, String, f32)>> {
        if self.mock_mode || (entity_facts.is_empty() && neighbor_facts.is_empty()) {
            return Ok(vec![]);
        }

        let mut facts_block = format!("Entity: {entity_name}\nKnown facts:\n");
        for f in entity_facts {
            facts_block.push_str(&format!("- \"{f}\"\n"));
        }

        facts_block.push_str("\nNeighbour facts:\n");
        for (name, facts) in neighbor_facts {
            let facts_str = facts.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
            facts_block.push_str(&format!("- {name}: {facts_str}\n"));
        }

        let system = "You are a knowledge graph inference agent. Given an entity, its known \
            facts, and facts about its neighbours, identify strongly inferable facts that are \
            missing. Return ONLY valid JSON with no markdown.";

        let user = format!(
            r#"{facts_block}
Identify missing facts about {entity_name} that can be strongly inferred from the above.
Examples: inherited surnames, nationality from parents, inverse relationships.

IMPORTANT: relation_type MUST be UPPER_SNAKE_CASE (e.g. PARENT_OF, CHILD_OF, SIBLING_OF, MARRIED_TO, SURNAME, FROM).
Object MUST be an entity name, not a bare attribute value.
Do NOT repeat facts that already appear above.

Return a JSON array:
[{{"relation_type": "STRING_UPPER", "object": "string (entity name)", "fact": "string", "confidence": 0.7}}]

Only include facts with confidence >= 0.7. Return [] if nothing can be strongly inferred."#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);

        #[derive(serde::Deserialize)]
        struct Inference {
            relation_type: String,
            object: String,
            fact: String,
            confidence: f32,
        }

        let inferences: Vec<Inference> = serde_json::from_str(text)
            .with_context(|| format!("failed to parse missing inferences — LLM returned: {text}"))?;
        Ok(inferences
            .into_iter()
            .map(|i| (i.relation_type, i.object, i.fact, i.confidence))
            .collect())
    }

    pub async fn generate_gap_questions(
        &self,
        entity_name: &str,
        known_facts: &[String],
        gap_types: &[String],
    ) -> Result<Vec<String>> {
        if self.mock_mode {
            return Ok(gap_types.iter()
                .map(|g| format!("What is {entity_name}'s {g}?"))
                .collect());
        }

        let system = "You are a knowledge gap analyst. Given known facts about an entity and identified \
            gap areas, generate specific questions that would fill those gaps.";

        let facts_list = if known_facts.is_empty() {
            "(none)".to_string()
        } else {
            known_facts.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
        };
        let gaps_list = gap_types.join(", ");

        let user = format!(
            "Entity: {entity_name}\n\
             Known facts:\n{facts_list}\n\n\
             Gap areas (relation types with no data): {gaps_list}\n\n\
             Generate 3-5 specific questions to fill these gaps. Return JSON:\n\
             {{\"questions\": [\"Where does {entity_name} live?\", ...]}}"
        );

        let text = self.call(system, &user, 512).await?;
        let text = clean_json(&text);
        let v: Value = serde_json::from_str(text)
            .with_context(|| format!("failed to parse gap questions — LLM returned: {text}"))?;
        let questions = v["questions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| q.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(questions)
    }

    pub async fn synthesise_answer(
        &self,
        question: &str,
        facts: &[crate::models::ContextFact],
        user_display_name: Option<&str>,
    ) -> Result<String> {
        if self.mock_mode {
            let fact_lines: Vec<&str> = facts.iter().map(|f| f.fact.as_str()).collect();
            return Ok(format!("Based on what I know: {}", fact_lines.join(". ")));
        }

        let user_ref = user_display_name.unwrap_or("Principal");
        let system = format!(
            "You are a helpful assistant answering questions from a knowledge graph. \
            Use only the provided facts to answer. If the facts don't contain enough information, \
            say so honestly. Be concise and direct. \
            In the knowledge graph, '{user_ref}' refers to the user — the person asking the question. \
            When answering, use 'you' instead of '{user_ref}' (e.g. 'Your sister is Alice' not '{user_ref}\\'s sister is Alice')."
        );

        let facts_block = if facts.is_empty() {
            "No relevant facts found.".to_string()
        } else {
            facts.iter()
                .map(|f| format!("- {} (confidence: {:.0}%)", f.fact, f.confidence * 100.0))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let user = format!(
            "Question: {question}\n\nKnown facts:\n{facts_block}\n\nAnswer the question based on these facts."
        );

        self.call(&system, &user, 1024).await
    }

    pub async fn extract_operations(
        &self,
        statement: &str,
        context: &crate::models::GraphContext,
    ) -> Result<crate::models::OperationsResult> {
        if self.mock_mode {
            // Simple mock: create nodes for capitalised words, edge between first two
            let names: Vec<String> = statement
                .split_whitespace()
                .filter(|w| w.chars().next().map_or(false, |c| c.is_uppercase()))
                .take(3)
                .map(|w| w.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
                .collect();

            let mut ops: Vec<crate::models::GraphOp> = names.iter().enumerate().map(|(i, name)| {
                crate::models::GraphOp::CreateNode {
                    node_ref: Some(format!("n{}", i + 1)),
                    name: name.clone(),
                    node_type: "unknown".to_string(),
                    properties: std::collections::HashMap::new(),
                }
            }).collect();

            if names.len() >= 2 {
                ops.push(crate::models::GraphOp::CreateEdge {
                    from: "n1".to_string(),
                    to: "n2".to_string(),
                    relation: "RELATED_TO".to_string(),
                    fact: statement.to_string(),
                    confidence: 0.8,
                });
            }

            return Ok(crate::models::OperationsResult { operations: ops });
        }

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
- "Widower" means spouse has died — model as MARRIED_TO edge (DECEASED on the spouse captures the temporal aspect)."#;

        let system = if self.extraction_prompt.is_empty() {
            system.to_string()
        } else {
            format!("{system}\n\nAdditional domain context:\n{}", self.extraction_prompt)
        };

        let user = format!(
            "Subgraph:\n{subgraph_json}\n\nNew statement: \"{statement}\""
        );

        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "operations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "enum": ["create_node", "update_node", "create_edge", "invalidate_edge"]
                            },
                            "ref": { "type": "string", "description": "Short reference for create_node (e.g. 'n1')" },
                            "name": { "type": "string" },
                            "type": { "type": "string", "enum": ["person", "place", "organization", "event", "concept", "content", "unknown"] },
                            "properties": { "type": "object", "additionalProperties": { "type": "string" } },
                            "id": { "type": "string", "description": "Existing node id for update_node" },
                            "set": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Properties to set on update_node" },
                            "from": { "type": "string", "description": "Source node id or ref for create_edge" },
                            "to": { "type": "string", "description": "Target node id or ref for create_edge" },
                            "relation": { "type": "string", "description": "Relation type (UPPER_SNAKE_CASE)" },
                            "fact": { "type": "string", "description": "Natural language fact for create_edge or invalidate_edge" },
                            "confidence": { "type": "number" },
                            "edge_id": { "type": "integer", "description": "Edge id for invalidate_edge" },
                            "reason": { "type": "string", "description": "Reason for invalidate_edge" }
                        },
                        "required": ["op"]
                    }
                }
            },
            "required": ["operations"]
        });

        if self.provider == LlmProvider::Anthropic {
            // Use tool use for guaranteed structured output
            tracing::debug!(system = %system, user = %user, "LLM tool request (extract_operations)");
            let result: crate::models::OperationsResult = self.call_anthropic_tool(
                &system, &user,
                "plan_graph_operations",
                "Plan the graph mutations needed to incorporate the new statement into the knowledge graph.",
                schema,
                self.max_tokens,
            ).await?;
            return Ok(result);
        }

        // Fallback for OpenAI or OAuth: use regular call + parse
        let text = self.call(&system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse operations result — LLM returned: {text}"))
    }

    pub async fn revise_operations(
        &self,
        original_ops: &crate::models::OperationsResult,
        additional_context: &crate::models::GraphContext,
    ) -> Result<crate::models::OperationsResult> {
        if self.mock_mode {
            return Ok(original_ops.clone());
        }

        let subgraph_json = additional_context.to_json();
        let ops_json = serde_json::to_string_pretty(&original_ops)?;

        let system = "You are a knowledge graph mutation planner. You previously planned operations \
            but now have additional graph context. Revise the operations if needed — for example, \
            convert create_node to update_node if you now see the entity already exists, or add \
            new edges based on the additional context. \
            Return ONLY valid JSON with no markdown, no explanation, no code fences.";

        let user = format!(
            r#"Original planned operations:
{ops_json}

Additional subgraph context discovered:
{subgraph_json}

Revise the operations. Return the COMPLETE final operations list (not just changes).
Use the same JSON format: {{"operations": [...]}}"#
        );

        let text = self.call(system, &user, self.max_tokens).await?;
        let text = clean_json(&text);
        serde_json::from_str(text)
            .with_context(|| format!("failed to parse revised operations — LLM returned: {text}"))
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if self.mock_mode {
            return Ok(pseudo_embed(text));
        }

        // When provider is OpenAI and an embedding model is configured, use OpenAI embeddings
        if self.provider == LlmProvider::OpenAI {
            if let Some(ref embedding_model) = self.openai_embedding_model {
                return self.embed_openai(text, embedding_model).await;
            }
        }

        // Default: Ollama embeddings
        self.embed_ollama(text).await
    }

    async fn embed_ollama(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", self.ollama_url);
        let body = serde_json::json!({
            "model": "nomic-embed-text",
            "prompt": text
        });

        match self.http.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let r: OllamaEmbeddingResponse = resp.json().await
                    .context("failed to parse Ollama embedding response")?;
                if r.embedding.len() == EMBEDDING_DIM {
                    return Ok(normalize(r.embedding));
                }
                let mut emb = r.embedding;
                emb.resize(EMBEDDING_DIM, 0.0);
                Ok(normalize(emb))
            }
            Ok(resp) => {
                let status = resp.status();
                tracing::warn!(
                    "PSEUDO-EMBEDDING: Ollama returned {status}, vector search will be degraded for '{}'",
                    &text[..text.len().min(50)]
                );
                Ok(pseudo_embed(text))
            }
            Err(e) => {
                tracing::warn!(
                    "PSEUDO-EMBEDDING: Ollama unavailable ({e}), vector search will be degraded for '{}'",
                    &text[..text.len().min(50)]
                );
                Ok(pseudo_embed(text))
            }
        }
    }

    async fn embed_openai(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        let api_key = self.openai_api_key.as_deref()
            .context("OPENAI_API_KEY required for OpenAI embeddings")?;

        let url = format!("{}/embeddings", self.openai_base_url);
        let req = OpenAIEmbeddingRequest {
            model: model.to_string(),
            input: text.to_string(),
        };

        let resp = self.http
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&req)
            .send()
            .await
            .context("failed to call OpenAI embeddings API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!("OpenAI embeddings error {status}: {body}, falling back to pseudo-embed");
            return Ok(pseudo_embed(text));
        }

        let r: OpenAIEmbeddingResponse = resp.json().await
            .context("failed to parse OpenAI embedding response")?;

        if let Some(data) = r.data.into_iter().next() {
            let mut emb = data.embedding;
            if emb.len() != EMBEDDING_DIM {
                emb.resize(EMBEDDING_DIM, 0.0);
            }
            Ok(normalize(emb))
        } else {
            tracing::warn!("Empty OpenAI embedding response, falling back to pseudo-embed");
            Ok(pseudo_embed(text))
        }
    }

    fn extract_entities_and_facts_mock(&self, statement: &str) -> Result<ExtractionResult> {
        // Try pattern-based extraction first for common sentence structures.
        if let Some(result) = self.try_pattern_extract(statement) {
            return Ok(result);
        }

        // Fallback: uppercase-word heuristic.
        let words: Vec<&str> = statement.split_whitespace().collect();
        let entities: Vec<ExtractedEntity> = words.iter()
            .filter(|w| w.chars().next().map_or(false, |c| c.is_uppercase()))
            .take(3)
            .map(|w| ExtractedEntity {
                name: w.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string(),
                entity_type: "unknown".to_string(),
                resolved: true,
                hint: None,
                content: None,
            })
            .collect();

        let facts = if entities.len() >= 2 {
            let subj = entities[0].name.clone();
            let obj = entities[1].name.clone();
            vec![ExtractedFact {
                subject: subj,
                relation_type: "RELATED_TO".to_string(),
                object: obj,
                fact: statement.to_string(),
                confidence: 0.8,
            }]
        } else {
            vec![]
        };

        Ok(ExtractionResult {
            entities,
            entity_updates: vec![],
            entity_attributes: vec![],
            explicit_facts: facts,
            implied_facts: vec![],
        })
    }

    /// Pattern-based mock extraction for common sentence structures.
    fn try_pattern_extract(&self, statement: &str) -> Option<ExtractionResult> {
        let patterns: &[(&str, &str, &str, &str)] = &[
            // (keyword, relation, subject_type, object_type)
            (" is a ", "IS_A", "person", "concept"),
            (" is an ", "IS_A", "person", "concept"),
            (" works at ", "WORKS_AT", "person", "organization"),
            (" works for ", "WORKS_AT", "person", "organization"),
            (" lives in ", "LIVES_IN", "person", "place"),
            (" moved to ", "LIVES_IN", "person", "place"),
            (" teaches at ", "WORKS_AT", "person", "organization"),
        ];

        for &(keyword, relation, subj_type, obj_type) in patterns {
            if let Some(idx) = statement.to_lowercase().find(keyword) {
                let subj = statement[..idx].trim().to_string();
                let obj = statement[idx + keyword.len()..].trim()
                    .trim_end_matches(|c: char| c == '.' || c == '!' || c == '?')
                    .to_string();
                if subj.is_empty() || obj.is_empty() {
                    continue;
                }
                let entities = vec![
                    ExtractedEntity {
                        name: subj.clone(),
                        entity_type: subj_type.to_string(),
                        resolved: true,
                        hint: None,
                        content: None,
                    },
                    ExtractedEntity {
                        name: obj.clone(),
                        entity_type: obj_type.to_string(),
                        resolved: true,
                        hint: None,
                        content: None,
                    },
                ];
                let facts = vec![ExtractedFact {
                    subject: subj,
                    relation_type: relation.to_string(),
                    object: obj,
                    fact: statement.to_string(),
                    confidence: 0.8,
                }];
                return Some(ExtractionResult {
                    entities,
                    entity_updates: vec![],
                    entity_attributes: vec![],
                    explicit_facts: facts,
                    implied_facts: vec![],
                });
            }
        }
        None
    }

    async fn call(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        tracing::debug!(system = %system, user = %user, max_tokens, "LLM request");
        let result = self.call_inner(system, user, max_tokens).await;
        match &result {
            Ok(response) => tracing::debug!(response = %response, "LLM response"),
            Err(e) => tracing::debug!(error = %e, "LLM error"),
        }
        result
    }

    async fn call_inner(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        match self.fixture_mode {
            FixtureMode::Replay => {
                let hash = fixtures::hash_request(system, user);
                let store = self.fixture_store.read().await;
                if let Some(fixture) = store.get(&hash) {
                    return Ok(fixture.response.clone());
                }
                anyhow::bail!("REPLAY mode: no fixture for hash {hash}. Run with EVAL_RECORD=1 first.");
            }
            FixtureMode::Record => {
                let response = self.call_real(system, user, max_tokens).await?;
                let hash = fixtures::hash_request(system, user);
                let fixture = LlmFixture {
                    request_hash: hash,
                    system: system[..system.len().min(500)].to_string(),
                    user: user[..user.len().min(200)].to_string(),
                    response: response.clone(),
                };
                let mut store = self.fixture_store.write().await;
                store.insert(fixture);
                store.save(&self.fixture_path);
                Ok(response)
            }
            FixtureMode::None => self.call_real(system, user, max_tokens).await,
        }
    }

    async fn call_real(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        match self.provider {
            LlmProvider::Anthropic => self.call_anthropic(system, user, max_tokens).await,
            LlmProvider::OpenAI => self.call_openai(system, user, max_tokens).await,
        }
    }

    async fn call_anthropic(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let (system, user) = match &self.auth {
            AnthropicAuth::OAuthToken(_) => (
                "You are Claude Code, Anthropic's official CLI for Claude.".to_string(),
                format!("{system}\n\n{user}"),
            ),
            _ => (system.to_string(), user.to_string()),
        };
        let req = AnthropicRequest {
            model: self.model.clone(),
            max_tokens,
            stream: false,
            system,
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user.to_string(),
            }],
            tools: None,
            tool_choice: None,
        };

        let url = match &self.auth {
            AnthropicAuth::OAuthToken(_) => "https://api.anthropic.com/v1/messages?beta=true",
            _ => "https://api.anthropic.com/v1/messages",
        };

        let mut builder = self.http
            .post(url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        builder = match &self.auth {
            AnthropicAuth::ApiKey(key) => builder.header("x-api-key", key),
            AnthropicAuth::OAuthToken(token) => builder
                .header("Authorization", format!("Bearer {token}"))
                .header("anthropic-beta", "oauth-2025-04-20,interleaved-thinking-2025-05-14,token-counting-2024-11-01"),
        };

        let req_body = serde_json::to_string(&req).unwrap_or_default();
        tracing::debug!(body = %req_body.chars().take(500).collect::<String>(), "Anthropic request body (truncated)");

        let resp = builder
            .json(&req)
            .send()
            .await
            .context("failed to call Anthropic API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {status}: {body}");
        }

        let r: AnthropicResponse = resp.json().await.context("failed to parse Anthropic response")?;
        // Extract text from the first text block
        for block in &r.content {
            if block.content_type.as_deref() == Some("text") {
                if let Some(ref text) = block.text {
                    return Ok(text.clone());
                }
            }
        }
        // Fallback: try first block's text field regardless of type
        r.content.into_iter().next()
            .and_then(|c| c.text)
            .context("empty Anthropic response")
    }

    async fn call_anthropic_tool<T: serde::de::DeserializeOwned>(
        &self,
        system: &str,
        user: &str,
        tool_name: &str,
        tool_description: &str,
        schema: serde_json::Value,
        max_tokens: u32,
    ) -> Result<T> {
        let (system, user) = match &self.auth {
            AnthropicAuth::OAuthToken(_) => (
                "You are Claude Code, Anthropic's official CLI for Claude.".to_string(),
                format!("{system}\n\n{user}"),
            ),
            _ => (system.to_string(), user.to_string()),
        };

        let req = AnthropicRequest {
            model: self.model.clone(),
            max_tokens,
            stream: false,
            system,
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user,
            }],
            tools: Some(vec![AnthropicTool {
                name: tool_name.to_string(),
                description: tool_description.to_string(),
                input_schema: schema,
            }]),
            tool_choice: Some(AnthropicToolChoice {
                choice_type: "tool".to_string(),
                name: Some(tool_name.to_string()),
            }),
        };

        let url = match &self.auth {
            AnthropicAuth::OAuthToken(_) => "https://api.anthropic.com/v1/messages?beta=true",
            _ => "https://api.anthropic.com/v1/messages",
        };

        let mut builder = self.http
            .post(url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        builder = match &self.auth {
            AnthropicAuth::ApiKey(key) => builder.header("x-api-key", key.as_str()),
            AnthropicAuth::OAuthToken(token) => builder
                .header("Authorization", format!("Bearer {token}"))
                .header("anthropic-beta", "oauth-2025-04-20,interleaved-thinking-2025-05-14,token-counting-2024-11-01"),
        };

        let resp = builder
            .json(&req)
            .send()
            .await
            .context("failed to call Anthropic API (tool use)")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {status}: {body}");
        }

        let r: AnthropicResponse = resp.json().await.context("failed to parse Anthropic tool response")?;
        for block in r.content {
            if block.content_type.as_deref() == Some("tool_use") {
                if let Some(input) = block.input {
                    tracing::debug!(tool = %tool_name, input = %input, "LLM tool response");
                    return serde_json::from_value(input)
                        .context("failed to parse tool use input");
                }
            }
        }
        anyhow::bail!("Anthropic tool response contained no tool_use block")
    }

    async fn call_openai(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let api_key = self.openai_api_key.as_deref()
            .context("OPENAI_API_KEY required for OpenAI provider")?;

        let req = OpenAIRequest {
            model: self.openai_model.clone(),
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

        let url = format!("{}/chat/completions", self.openai_base_url);
        let resp = self.http
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&req)
            .send()
            .await
            .context("failed to call OpenAI API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {status}: {body}");
        }

        let r: OpenAIResponse = resp.json().await.context("failed to parse OpenAI response")?;
        r.choices.into_iter().next()
            .and_then(|c| c.message.content)
            .context("empty OpenAI response")
    }
}

fn clean_json(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    let s = s.trim();

    // Find whichever starts first — { or [ — so we correctly handle both objects and arrays
    let obj_start = s.find('{');
    let arr_start = s.find('[');

    let (start, open, close) = match (obj_start, arr_start) {
        (Some(o), Some(a)) if a < o => (a, b'[', b']'),
        (Some(o), _) => (o, b'{', b'}'),
        (None, Some(a)) => (a, b'[', b']'),
        (None, None) => return s,
    };

    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes[start..].iter().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return &s[start..start + i + 1];
            }
        }
    }
    s
}

pub fn pseudo_embed(text: &str) -> Vec<f32> {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let hash = hasher.finalize();
    let seed = u64::from_le_bytes(hash[..8].try_into().unwrap());
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let v: Vec<f32> = (0..EMBEDDING_DIM).map(|_| rng.gen_range(-1.0f32..1.0f32)).collect();
    normalize(v)
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

// ---- Trait implementation ----

use crate::llm_service::LlmService;

#[async_trait]
impl LlmService for LlmClient {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        LlmClient::embed(self, text).await
    }

    async fn extract_operations(
        &self,
        statement: &str,
        context: &crate::models::GraphContext,
    ) -> Result<crate::models::OperationsResult> {
        LlmClient::extract_operations(self, statement, context).await
    }

    async fn revise_operations(
        &self,
        original_ops: &crate::models::OperationsResult,
        additional_context: &crate::models::GraphContext,
    ) -> Result<crate::models::OperationsResult> {
        LlmClient::revise_operations(self, original_ops, additional_context).await
    }

    async fn resolve_entities(
        &self,
        extracted: &ExtractedEntity,
        candidate: &EntityRow,
        candidate_facts: &[String],
    ) -> Result<(bool, f32)> {
        LlmClient::resolve_entities(self, extracted, candidate, candidate_facts).await
    }

    async fn resolve_entities_batch(
        &self,
        pairs: &[(String, String, String, String, Vec<String>)],
    ) -> Result<Vec<(usize, bool, f32)>> {
        LlmClient::resolve_entities_batch(self, pairs).await
    }

    async fn classify_edge(
        &self,
        existing_fact: &str,
        new_fact: &str,
        relation_type: &str,
    ) -> Result<(EdgeClassification, f32)> {
        LlmClient::classify_edge(self, existing_fact, new_fact, relation_type).await
    }

    async fn classify_edges_batch(
        &self,
        existing_facts: &[&str],
        new_fact: &str,
        relation_type: &str,
    ) -> Result<Vec<(usize, EdgeClassification, f32)>> {
        LlmClient::classify_edges_batch(self, existing_facts, new_fact, relation_type).await
    }

    async fn discover_link(
        &self,
        a: &EntityRow,
        b: &EntityRow,
        a_facts: &[String],
        b_facts: &[String],
    ) -> Result<Option<(String, String, f32)>> {
        LlmClient::discover_link(self, a, b, a_facts, b_facts).await
    }

    async fn synthesise_answer(
        &self,
        question: &str,
        facts: &[crate::models::ContextFact],
        user_display_name: Option<&str>,
    ) -> Result<String> {
        LlmClient::synthesise_answer(self, question, facts, user_display_name).await
    }

    async fn extract_entities_and_facts(
        &self,
        statement: &str,
    ) -> Result<ExtractionResult> {
        LlmClient::extract_entities_and_facts(self, statement).await
    }

    async fn extract_entities_and_facts_with_context(
        &self,
        statement: &str,
        context: &crate::models::GraphContext,
    ) -> Result<ExtractionResult> {
        LlmClient::extract_entities_and_facts_with_context(self, statement, context).await
    }

    async fn infer_additional_facts(
        &self,
        extracted_facts: &[ExtractedFact],
        entity_context: &[(String, Vec<String>)],
    ) -> Result<crate::models::EnrichmentResult> {
        LlmClient::infer_additional_facts(self, extracted_facts, entity_context).await
    }

    async fn find_missing_inferences(
        &self,
        entity_name: &str,
        entity_facts: &[String],
        neighbor_facts: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String, String, f32)>> {
        LlmClient::find_missing_inferences(self, entity_name, entity_facts, neighbor_facts).await
    }

    async fn generate_gap_questions(
        &self,
        entity_name: &str,
        known_facts: &[String],
        gap_types: &[String],
    ) -> Result<Vec<String>> {
        LlmClient::generate_gap_questions(self, entity_name, known_facts, gap_types).await
    }
}
