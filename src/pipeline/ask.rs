use anyhow::Result;

use crate::graph_backend::GraphBackend;
use crate::models::{AskRequest, AskResponse, ContextRequest};
use crate::state::AppState;

pub async fn ask(
    state: &AppState,
    graph: &dyn GraphBackend,
    req: AskRequest,
) -> Result<AskResponse> {
    let limit = req.limit.unwrap_or(state.config.default_context_limit);

    let ctx_req = ContextRequest {
        query: req.question.clone(),
        limit: Some(limit),
        max_hops: None,
        memory_tier_filter: None,
        graph: None,
    };

    let ctx = super::context::context(state, graph, ctx_req, None).await?;

    let answer = state.llm.synthesise_answer(&req.question, &ctx.facts).await?;

    Ok(AskResponse {
        answer,
        facts: if req.verbose { Some(ctx.facts) } else { None },
    })
}
