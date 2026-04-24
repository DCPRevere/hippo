//! Test doubles for unit testing pipeline logic.
//!
//! Provides `FakeLlm` — a configurable `LlmService` implementation that returns
//! pre-programmed responses without making any network calls.

use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use crate::llm_service::LlmService;
use crate::math;
use crate::models::{
    ContextFact, EdgeClassification, EntityRow, ExtractedEntity, GraphContext, OperationsResult,
};

/// A configurable LLM test double.
///
/// Each method pops from its respective queue. When a queue is empty, a sensible
/// default is returned (empty results, `Unrelated` classification, etc.).
///
/// Use the builder methods (`with_operations`, `with_classification`, …) to
/// set up the responses a test expects.
#[allow(clippy::type_complexity)]
pub struct FakeLlm {
    operations: Mutex<VecDeque<OperationsResult>>,
    revised_operations: Mutex<VecDeque<OperationsResult>>,
    classifications: Mutex<VecDeque<(EdgeClassification, f32)>>,
    entity_resolutions: Mutex<VecDeque<(bool, f32)>>,
    batch_entity_resolutions: Mutex<VecDeque<Vec<(usize, bool, f32)>>>,
    link_discoveries: Mutex<VecDeque<Option<(String, String, f32)>>>,
    answers: Mutex<VecDeque<String>>,
    missing_inferences: Mutex<VecDeque<Vec<(String, String, String, f32)>>>,
    gap_questions: Mutex<VecDeque<Vec<String>>>,
}

impl Default for FakeLlm {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeLlm {
    pub fn new() -> Self {
        Self {
            operations: Mutex::new(VecDeque::new()),
            revised_operations: Mutex::new(VecDeque::new()),
            classifications: Mutex::new(VecDeque::new()),
            entity_resolutions: Mutex::new(VecDeque::new()),
            batch_entity_resolutions: Mutex::new(VecDeque::new()),
            link_discoveries: Mutex::new(VecDeque::new()),
            answers: Mutex::new(VecDeque::new()),
            missing_inferences: Mutex::new(VecDeque::new()),
            gap_questions: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueue an `OperationsResult` for the next `extract_operations` call.
    pub fn with_operations(self, ops: OperationsResult) -> Self {
        self.operations.lock().unwrap().push_back(ops);
        self
    }

    /// Enqueue a revised `OperationsResult` for the next `revise_operations` call.
    pub fn with_revised_operations(self, ops: OperationsResult) -> Self {
        self.revised_operations.lock().unwrap().push_back(ops);
        self
    }

    /// Enqueue a classification result for the next `classify_edge` call.
    pub fn with_classification(self, class: EdgeClassification, confidence: f32) -> Self {
        self.classifications
            .lock()
            .unwrap()
            .push_back((class, confidence));
        self
    }

    /// Enqueue an entity resolution result for the next `resolve_entities` call.
    pub fn with_entity_resolution(self, same: bool, confidence: f32) -> Self {
        self.entity_resolutions
            .lock()
            .unwrap()
            .push_back((same, confidence));
        self
    }

    /// Enqueue a link discovery result.
    pub fn with_link_discovery(self, link: Option<(String, String, f32)>) -> Self {
        self.link_discoveries.lock().unwrap().push_back(link);
        self
    }

    /// Enqueue an answer for the next `synthesise_answer` call.
    pub fn with_answer(self, answer: String) -> Self {
        self.answers.lock().unwrap().push_back(answer);
        self
    }

    /// Enqueue missing inferences.
    pub fn with_missing_inferences(self, inferences: Vec<(String, String, String, f32)>) -> Self {
        self.missing_inferences
            .lock()
            .unwrap()
            .push_back(inferences);
        self
    }

    /// Enqueue gap questions.
    pub fn with_gap_questions(self, questions: Vec<String>) -> Self {
        self.gap_questions.lock().unwrap().push_back(questions);
        self
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl LlmService for FakeLlm {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(math::pseudo_embed(text))
    }

    async fn extract_operations(
        &self,
        _statement: &str,
        _context: &GraphContext,
    ) -> Result<OperationsResult> {
        Ok(self
            .operations
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| OperationsResult { operations: vec![] }))
    }

    async fn revise_operations(
        &self,
        original_ops: &OperationsResult,
        _additional_context: &GraphContext,
    ) -> Result<OperationsResult> {
        Ok(self
            .revised_operations
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| original_ops.clone()))
    }

    async fn resolve_entities(
        &self,
        _extracted: &ExtractedEntity,
        _candidate: &EntityRow,
        _candidate_facts: &[String],
    ) -> Result<(bool, f32)> {
        Ok(self
            .entity_resolutions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((false, 0.5)))
    }

    async fn resolve_entities_batch(
        &self,
        pairs: &[(String, String, String, String, Vec<String>)],
    ) -> Result<Vec<(usize, bool, f32)>> {
        Ok(self
            .batch_entity_resolutions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                pairs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| (i, false, 0.5))
                    .collect()
            }))
    }

    async fn classify_edge(
        &self,
        _existing_fact: &str,
        _new_fact: &str,
        _relation_type: &str,
    ) -> Result<(EdgeClassification, f32)> {
        Ok(self
            .classifications
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((EdgeClassification::Unrelated, 0.5)))
    }

    async fn discover_link(
        &self,
        _a: &EntityRow,
        _b: &EntityRow,
        _a_facts: &[String],
        _b_facts: &[String],
    ) -> Result<Option<(String, String, f32)>> {
        Ok(self.link_discoveries.lock().unwrap().pop_front().flatten())
    }

    async fn synthesise_answer(
        &self,
        _question: &str,
        facts: &[ContextFact],
        _user_display_name: Option<&str>,
    ) -> Result<String> {
        Ok(self.answers.lock().unwrap().pop_front().unwrap_or_else(|| {
            facts
                .iter()
                .map(|f| f.fact.clone())
                .collect::<Vec<_>>()
                .join("; ")
        }))
    }

    async fn identify_missing_context(
        &self,
        _question: &str,
        _facts: &[ContextFact],
    ) -> Result<Vec<String>> {
        Ok(vec![])
    }

    async fn find_missing_inferences(
        &self,
        _entity_name: &str,
        _entity_facts: &[String],
        _neighbor_facts: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String, String, f32)>> {
        Ok(self
            .missing_inferences
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default())
    }

    async fn generate_gap_questions(
        &self,
        _entity_name: &str,
        _known_facts: &[String],
        _gap_types: &[String],
    ) -> Result<Vec<String>> {
        Ok(self
            .gap_questions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default())
    }
}
