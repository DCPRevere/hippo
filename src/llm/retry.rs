//! Retry decorator for `LlmService` — wraps any provider with jittered
//! exponential backoff on transient failures (rate limits, server errors).
//!
//! Today's `LlmClient` returns `anyhow::Error` for everything, so we can't
//! distinguish transient from permanent failures by type. The decorator
//! retries any error matching a string pattern (rate-limit hints, 429,
//! 5xx). A future refactor of LlmClient to return a typed error enum will
//! let this be cleaner; until then, the string match is the pragmatic
//! retry signal.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::llm_service::LlmService;
use crate::models::{
    ContextFact, EdgeClassification, EntityRow, ExtractedEntity, GraphContext, OperationsResult,
};

/// Wraps an `LlmService`, retrying transient failures with jittered
/// exponential backoff.
pub struct RetryingLlm {
    inner: Arc<dyn LlmService>,
    max_attempts: u32,
    base_delay_ms: u64,
}

impl RetryingLlm {
    pub fn new(inner: Arc<dyn LlmService>) -> Self {
        Self {
            inner,
            max_attempts: 3,
            base_delay_ms: 250,
        }
    }

    pub fn with_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    pub fn with_base_delay_ms(mut self, base_delay_ms: u64) -> Self {
        self.base_delay_ms = base_delay_ms;
        self
    }

    async fn run<F, T, Fut>(&self, mut op: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut attempt: u32 = 0;
        loop {
            match op().await {
                Ok(t) => return Ok(t),
                Err(e) if attempt + 1 < self.max_attempts && is_transient(&e) => {
                    attempt += 1;
                    let backoff = self.backoff_ms(attempt);
                    tracing::warn!(
                        attempt,
                        backoff_ms = backoff,
                        error = %e,
                        "llm call failed transiently, retrying",
                    );
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn backoff_ms(&self, attempt: u32) -> u64 {
        // Exponential with jitter: base * 2^(attempt-1) ± up to 25%.
        let base = self.base_delay_ms.saturating_mul(1u64 << attempt.min(8));
        let jitter_range = base / 4;
        let jitter = simple_jitter(attempt) % jitter_range.max(1);
        base.saturating_add(jitter)
    }
}

/// Returns true when the error looks transient — rate limits, 429, 5xx,
/// connection-reset patterns. Best-effort string match against the error
/// chain.
fn is_transient(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}").to_lowercase();
    s.contains("429")
        || s.contains("rate limit")
        || s.contains("rate-limit")
        || s.contains("too many requests")
        || s.contains("503")
        || s.contains("502")
        || s.contains("504")
        || s.contains("timeout")
        || s.contains("connection reset")
        || s.contains("temporarily unavailable")
}

/// Tiny deterministic-ish jitter source. Uses the system time so retries
/// across many concurrent failures don't all back off in lockstep, but
/// avoids the rand crate (already a dep but kept lean here).
fn simple_jitter(attempt: u32) -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos.wrapping_add(attempt as u64 * 7919)
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl LlmService for RetryingLlm {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.run(|| self.inner.embed(text)).await
    }

    async fn extract_operations(
        &self,
        statement: &str,
        context: &GraphContext,
    ) -> Result<OperationsResult> {
        self.run(|| self.inner.extract_operations(statement, context))
            .await
    }

    async fn revise_operations(
        &self,
        original_ops: &OperationsResult,
        additional_context: &GraphContext,
    ) -> Result<OperationsResult> {
        self.run(|| {
            self.inner
                .revise_operations(original_ops, additional_context)
        })
        .await
    }

    async fn resolve_entities(
        &self,
        extracted: &ExtractedEntity,
        candidate: &EntityRow,
        candidate_facts: &[String],
    ) -> Result<(bool, f32)> {
        self.run(|| {
            self.inner
                .resolve_entities(extracted, candidate, candidate_facts)
        })
        .await
    }

    async fn resolve_entities_batch(
        &self,
        pairs: &[(String, String, String, String, Vec<String>)],
    ) -> Result<Vec<(usize, bool, f32)>> {
        self.run(|| self.inner.resolve_entities_batch(pairs)).await
    }

    async fn classify_edge(
        &self,
        existing_fact: &str,
        new_fact: &str,
        relation_type: &str,
    ) -> Result<(EdgeClassification, f32)> {
        self.run(|| {
            self.inner
                .classify_edge(existing_fact, new_fact, relation_type)
        })
        .await
    }

    async fn discover_link(
        &self,
        a: &EntityRow,
        b: &EntityRow,
        a_facts: &[String],
        b_facts: &[String],
    ) -> Result<Option<(String, String, f32)>> {
        self.run(|| self.inner.discover_link(a, b, a_facts, b_facts))
            .await
    }

    async fn synthesise_answer(
        &self,
        question: &str,
        facts: &[ContextFact],
        user_display_name: Option<&str>,
    ) -> Result<String> {
        self.run(|| {
            self.inner
                .synthesise_answer(question, facts, user_display_name)
        })
        .await
    }

    async fn identify_missing_context(
        &self,
        question: &str,
        facts: &[ContextFact],
    ) -> Result<Vec<String>> {
        self.run(|| self.inner.identify_missing_context(question, facts))
            .await
    }

    async fn find_missing_inferences(
        &self,
        entity_name: &str,
        entity_facts: &[String],
        neighbor_facts: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String, String, f32)>> {
        self.run(|| {
            self.inner
                .find_missing_inferences(entity_name, entity_facts, neighbor_facts)
        })
        .await
    }

    async fn generate_gap_questions(
        &self,
        entity_name: &str,
        known_facts: &[String],
        gap_types: &[String],
    ) -> Result<Vec<String>> {
        self.run(|| {
            self.inner
                .generate_gap_questions(entity_name, known_facts, gap_types)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A test LlmService that fails N times then succeeds. We exercise it
    /// through the embed() method; other methods route through the same
    /// retry path so testing one is sufficient.
    struct FlakyLlm {
        fails_remaining: AtomicUsize,
        attempts: AtomicUsize,
        error_for_failures: Mutex<String>,
    }

    impl FlakyLlm {
        fn new(fail_count: usize, error_text: &str) -> Self {
            Self {
                fails_remaining: AtomicUsize::new(fail_count),
                attempts: AtomicUsize::new(0),
                error_for_failures: Mutex::new(error_text.to_string()),
            }
        }
    }

    #[async_trait]
    impl LlmService for FlakyLlm {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            self.attempts.fetch_add(1, Ordering::Relaxed);
            let prev = self.fails_remaining.fetch_sub(1, Ordering::Relaxed);
            if prev > 0 {
                let msg = self.error_for_failures.lock().unwrap().clone();
                Err(anyhow::anyhow!("{msg}"))
            } else {
                self.fails_remaining.store(0, Ordering::Relaxed);
                Ok(vec![0.1, 0.2, 0.3])
            }
        }
        async fn extract_operations(&self, _: &str, _: &GraphContext) -> Result<OperationsResult> {
            unimplemented!()
        }
        async fn revise_operations(
            &self,
            _: &OperationsResult,
            _: &GraphContext,
        ) -> Result<OperationsResult> {
            unimplemented!()
        }
        async fn resolve_entities(
            &self,
            _: &ExtractedEntity,
            _: &EntityRow,
            _: &[String],
        ) -> Result<(bool, f32)> {
            unimplemented!()
        }
        async fn resolve_entities_batch(
            &self,
            _: &[(String, String, String, String, Vec<String>)],
        ) -> Result<Vec<(usize, bool, f32)>> {
            unimplemented!()
        }
        async fn classify_edge(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(EdgeClassification, f32)> {
            unimplemented!()
        }
        async fn discover_link(
            &self,
            _: &EntityRow,
            _: &EntityRow,
            _: &[String],
            _: &[String],
        ) -> Result<Option<(String, String, f32)>> {
            unimplemented!()
        }
        async fn synthesise_answer(
            &self,
            _: &str,
            _: &[ContextFact],
            _: Option<&str>,
        ) -> Result<String> {
            unimplemented!()
        }
        async fn identify_missing_context(
            &self,
            _: &str,
            _: &[ContextFact],
        ) -> Result<Vec<String>> {
            unimplemented!()
        }
        async fn find_missing_inferences(
            &self,
            _: &str,
            _: &[String],
            _: &[(String, Vec<String>)],
        ) -> Result<Vec<(String, String, String, f32)>> {
            unimplemented!()
        }
        async fn generate_gap_questions(
            &self,
            _: &str,
            _: &[String],
            _: &[String],
        ) -> Result<Vec<String>> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn retries_on_transient_error() {
        let flaky = Arc::new(FlakyLlm::new(2, "HTTP 429 too many requests"));
        let retrying = RetryingLlm::new(flaky.clone() as Arc<dyn LlmService>)
            .with_attempts(5)
            .with_base_delay_ms(1);
        let result = retrying.embed("hello").await;
        assert!(result.is_ok());
        assert_eq!(flaky.attempts.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn does_not_retry_permanent_error() {
        let flaky = Arc::new(FlakyLlm::new(5, "invalid api key"));
        let retrying = RetryingLlm::new(flaky.clone() as Arc<dyn LlmService>)
            .with_attempts(5)
            .with_base_delay_ms(1);
        let result = retrying.embed("hello").await;
        assert!(result.is_err());
        // Permanent error means one attempt only.
        assert_eq!(flaky.attempts.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let flaky = Arc::new(FlakyLlm::new(10, "rate limit exceeded"));
        let retrying = RetryingLlm::new(flaky.clone() as Arc<dyn LlmService>)
            .with_attempts(3)
            .with_base_delay_ms(1);
        let result = retrying.embed("hello").await;
        assert!(result.is_err());
        assert_eq!(flaky.attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn transient_classification() {
        assert!(is_transient(&anyhow::anyhow!("HTTP 429")));
        assert!(is_transient(&anyhow::anyhow!("rate limit exceeded")));
        assert!(is_transient(&anyhow::anyhow!("503 Service Unavailable")));
        assert!(is_transient(&anyhow::anyhow!("connection reset by peer")));
        assert!(!is_transient(&anyhow::anyhow!("invalid api key")));
        assert!(!is_transient(&anyhow::anyhow!("400 bad request")));
    }
}
