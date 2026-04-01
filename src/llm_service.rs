use anyhow::Result;
use async_trait::async_trait;

use crate::models::{
    ContextFact, EdgeClassification, EntityRow,
    ExtractedEntity, GraphContext, OperationsResult,
};

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait LlmService: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    async fn extract_operations(
        &self,
        statement: &str,
        context: &GraphContext,
    ) -> Result<OperationsResult>;

    async fn revise_operations(
        &self,
        original_ops: &OperationsResult,
        additional_context: &GraphContext,
    ) -> Result<OperationsResult>;

    async fn resolve_entities(
        &self,
        extracted: &ExtractedEntity,
        candidate: &EntityRow,
        candidate_facts: &[String],
    ) -> Result<(bool, f32)>;

    async fn resolve_entities_batch(
        &self,
        pairs: &[(String, String, String, String, Vec<String>)],
    ) -> Result<Vec<(usize, bool, f32)>>;

    async fn classify_edge(
        &self,
        existing_fact: &str,
        new_fact: &str,
        relation_type: &str,
    ) -> Result<(EdgeClassification, f32)>;

    async fn discover_link(
        &self,
        a: &EntityRow,
        b: &EntityRow,
        a_facts: &[String],
        b_facts: &[String],
    ) -> Result<Option<(String, String, f32)>>;

    async fn synthesise_answer(
        &self,
        question: &str,
        facts: &[ContextFact],
        user_display_name: Option<&str>,
    ) -> Result<String>;

    async fn find_missing_inferences(
        &self,
        entity_name: &str,
        entity_facts: &[String],
        neighbor_facts: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String, String, f32)>>;

    async fn generate_gap_questions(
        &self,
        entity_name: &str,
        known_facts: &[String],
        gap_types: &[String],
    ) -> Result<Vec<String>>;
}
