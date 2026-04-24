use std::sync::Arc;

use wasm_bindgen::prelude::*;

use hippo::config::Config;
use hippo::graph_backend::GraphBackend;
use hippo::in_memory_graph::InMemoryGraph;
use hippo::models::{AskRequest, RememberRequest};
use hippo::state::AppState;

mod wasm_llm;
use wasm_llm::WasmLlmClient;

#[wasm_bindgen]
pub struct Hippo {
    state: Arc<AppState>,
    graph: Arc<dyn GraphBackend>,
}

#[wasm_bindgen]
impl Hippo {
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: &str) -> Result<Hippo, JsValue> {
        #[derive(serde::Deserialize)]
        struct WasmConfig {
            api_key: String,
            model: Option<String>,
            embedding_model: Option<String>,
            base_url: Option<String>,
        }

        let wasm_config: WasmConfig = serde_json::from_str(config_json)
            .map_err(|e| JsValue::from_str(&format!("invalid config: {e}")))?;

        let config = Config::for_wasm(
            wasm_config.api_key.clone(),
            wasm_config.model.clone(),
            wasm_config.embedding_model.clone(),
        );

        let llm: Arc<dyn hippo::llm_service::LlmService> = Arc::new(WasmLlmClient::new(
            wasm_config.api_key,
            wasm_config
                .base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            wasm_config
                .model
                .unwrap_or_else(|| "gpt-5.4-mini".to_string()),
            wasm_config
                .embedding_model
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
        ));

        let graph_name = config.graph.name.clone();
        let graph =
            Arc::new(InMemoryGraph::new(&graph_name)) as Arc<dyn GraphBackend>;

        let state = AppState::for_test(llm, config);

        Ok(Hippo {
            state: Arc::new(state),
            graph,
        })
    }

    pub async fn remember(
        &self,
        statement: &str,
        source_agent: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let req = RememberRequest {
            statement: statement.to_string(),
            source_agent,
            source_credibility_hint: None,
            graph: None,
            ttl_secs: None,
        };

        let result = hippo::pipeline::remember::remember(
            &self.state,
            &*self.graph,
            req,
            None,
            None,
        )
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&result)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    pub async fn ask(&self, question: &str) -> Result<JsValue, JsValue> {
        let req = AskRequest {
            question: question.to_string(),
            limit: None,
            graph: None,
            verbose: false,
            max_iterations: 1,
        };

        let result =
            hippo::pipeline::ask::ask(&self.state, &*self.graph, req, None, None)
                .await
                .map_err(|e| JsValue::from_str(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&result)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    pub async fn context(
        &self,
        query: &str,
    ) -> Result<JsValue, JsValue> {
        let ctx = hippo::pipeline::remember::gather_pre_extraction_context(
            &self.state, &*self.graph, query, None,
        )
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&ctx)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    pub async fn stats(&self) -> Result<JsValue, JsValue> {
        let stats = self
            .graph
            .graph_stats()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&stats)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    pub async fn export_graph(&self) -> Result<JsValue, JsValue> {
        let entities = self
            .graph
            .dump_all_entities()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let edges = self
            .graph
            .dump_all_edges()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        let export = serde_json::json!({
            "entities": entities,
            "edges": edges,
        });

        serde_wasm_bindgen::to_value(&export)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
}
