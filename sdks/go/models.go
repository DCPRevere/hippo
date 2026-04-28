package hippo

import "strings"

// LlmUsage tracks token and call counts for LLM/embedding operations within a
// pipeline run.
type LlmUsage struct {
	LlmCalls     int `json:"llm_calls"`
	EmbedCalls   int `json:"embed_calls"`
	InputTokens  int `json:"input_tokens"`
	OutputTokens int `json:"output_tokens"`
}

// ScoringParams overrides the default weights used when scoring context.
type ScoringParams struct {
	WRelevance  float32 `json:"w_relevance"`
	WConfidence float32 `json:"w_confidence"`
	WRecency    float32 `json:"w_recency"`
	WSalience   float32 `json:"w_salience"`
	MMRLambda   float32 `json:"mmr_lambda"`
}

// OpExecutionTrace records the outcome of a single graph operation.
type OpExecutionTrace struct {
	Op      string  `json:"op"`
	Outcome string  `json:"outcome"`
	Details *string `json:"details,omitempty"`
}

// RememberTrace contains the operations the LLM proposed and the execution
// outcomes for each.
type RememberTrace struct {
	Operations         []map[string]any   `json:"operations"`
	RevisedOperations  []map[string]any   `json:"revised_operations,omitempty"`
	Execution          []OpExecutionTrace `json:"execution"`
}

// RememberRequest is the body for POST /api/remember.
type RememberRequest struct {
	Statement             string   `json:"statement"`
	SourceAgent           *string  `json:"source_agent,omitempty"`
	SourceCredibilityHint *float32 `json:"source_credibility_hint,omitempty"`
	Graph                 *string  `json:"graph,omitempty"`
	TTLSecs               *int     `json:"ttl_secs,omitempty"`
}

// RememberResponse is the response from POST /api/remember.
type RememberResponse struct {
	EntitiesCreated           int            `json:"entities_created"`
	EntitiesResolved          int            `json:"entities_resolved"`
	FactsWritten              int            `json:"facts_written"`
	ContradictionsInvalidated int            `json:"contradictions_invalidated"`
	Usage                     *LlmUsage      `json:"usage,omitempty"`
	Trace                     *RememberTrace `json:"trace,omitempty"`
}

// BatchRememberRequest is the body for POST /api/remember/batch.
type BatchRememberRequest struct {
	Statements  []string `json:"statements"`
	SourceAgent *string  `json:"source_agent,omitempty"`
	Parallel    *bool    `json:"parallel,omitempty"`
	Graph       *string  `json:"graph,omitempty"`
	TTLSecs     *int     `json:"ttl_secs,omitempty"`
}

// BatchRememberResult is one entry in the batch response.
type BatchRememberResult struct {
	Statement       string  `json:"statement"`
	OK              bool    `json:"ok"`
	FactsWritten    *int    `json:"facts_written,omitempty"`
	EntitiesCreated *int    `json:"entities_created,omitempty"`
	Error           *string `json:"error,omitempty"`
}

// BatchRememberResponse is the response from POST /api/remember/batch.
type BatchRememberResponse struct {
	Total     int                   `json:"total"`
	Succeeded int                   `json:"succeeded"`
	Failed    int                   `json:"failed"`
	Results   []BatchRememberResult `json:"results"`
}

// ContextRequest is the body for POST /api/context.
type ContextRequest struct {
	Query             string         `json:"query"`
	Limit             *int           `json:"limit,omitempty"`
	MaxHops           *int           `json:"max_hops,omitempty"`
	MemoryTierFilter  *string        `json:"memory_tier_filter,omitempty"`
	Graph             *string        `json:"graph,omitempty"`
	At                *string        `json:"at,omitempty"`
	Scoring           *ScoringParams `json:"scoring,omitempty"`
}

// ContextFact is one fact returned by /api/context.
type ContextFact struct {
	Fact         string   `json:"fact"`
	Subject      string   `json:"subject"`
	RelationType string   `json:"relation_type"`
	Object       string   `json:"object"`
	Confidence   float32  `json:"confidence"`
	Salience     int64    `json:"salience"`
	ValidAt      string   `json:"valid_at"`
	EdgeID       int64    `json:"edge_id"`
	Hops         int      `json:"hops"`
	SourceAgents []string `json:"source_agents"`
	MemoryTier   string   `json:"memory_tier"`
}

// ContextResponse is the response from POST /api/context.
type ContextResponse struct {
	Facts []ContextFact `json:"facts"`
}

// AskRequest is the body for POST /api/ask.
type AskRequest struct {
	Question      string  `json:"question"`
	Limit         *int    `json:"limit,omitempty"`
	Graph         *string `json:"graph,omitempty"`
	Verbose       *bool   `json:"verbose,omitempty"`
	MaxIterations *int    `json:"max_iterations,omitempty"`
}

// AskResponse is the response from POST /api/ask.
type AskResponse struct {
	Answer     string        `json:"answer"`
	Facts      []ContextFact `json:"facts,omitempty"`
	Iterations int           `json:"iterations"`
}

// RetractRequest is the body for POST /api/retract.
type RetractRequest struct {
	EdgeID int64   `json:"edge_id"`
	Reason *string `json:"reason,omitempty"`
	Graph  *string `json:"graph,omitempty"`
}

// RetractResponse is the response from POST /api/retract.
type RetractResponse struct {
	EdgeID int64   `json:"edge_id"`
	Reason *string `json:"reason,omitempty"`
}

// CorrectRequest is the body for POST /api/correct.
type CorrectRequest struct {
	EdgeID      int64   `json:"edge_id"`
	Statement   string  `json:"statement"`
	Reason      *string `json:"reason,omitempty"`
	SourceAgent *string `json:"source_agent,omitempty"`
	Graph       *string `json:"graph,omitempty"`
}

// CorrectResponse is the response from POST /api/correct.
type CorrectResponse struct {
	RetractedEdgeID int64            `json:"retracted_edge_id"`
	Reason          *string          `json:"reason,omitempty"`
	Remember        RememberResponse `json:"remember"`
}

// CreateUserRequest is the body for POST /api/admin/users.
type CreateUserRequest struct {
	UserID      string   `json:"user_id"`
	DisplayName string   `json:"display_name"`
	Role        *string  `json:"role,omitempty"`
	Graphs      []string `json:"graphs,omitempty"`
}

// CreateUserResponse is the response from POST /api/admin/users.
type CreateUserResponse struct {
	UserID string `json:"user_id"`
	APIKey string `json:"api_key"`
}

// User represents a user in the admin list.
type User struct {
	UserID      string   `json:"user_id"`
	DisplayName string   `json:"display_name"`
	Role        string   `json:"role"`
	Graphs      []string `json:"graphs"`
	KeyCount    int      `json:"key_count"`
}

// ListUsersResponse is the response from GET /api/admin/users.
type ListUsersResponse struct {
	Users []User `json:"users"`
}

// CreateKeyRequest is the body for POST /api/admin/users/{user_id}/keys.
type CreateKeyRequest struct {
	Label string `json:"label"`
}

// CreateKeyResponse is the response from POST /api/admin/users/{user_id}/keys.
type CreateKeyResponse struct {
	UserID string `json:"user_id"`
	Label  string `json:"label"`
	APIKey string `json:"api_key"`
}

// Key represents an API key entry in a list.
type Key struct {
	Label     string `json:"label"`
	CreatedAt string `json:"created_at"`
}

// ListKeysResponse is the response from GET /api/admin/users/{user_id}/keys.
type ListKeysResponse struct {
	Keys []Key `json:"keys"`
}

// HealthResponse is the response from GET /health.
type HealthResponse struct {
	Status string `json:"status"`
	Graph  string `json:"graph"`
}

// GraphsListResponse is the response from GET /api/graphs.
type GraphsListResponse struct {
	Default string   `json:"default"`
	Graphs  []string `json:"graphs"`
}

// AuditEntry is one row of the admin audit log.
type AuditEntry struct {
	ID        string `json:"id"`
	UserID    string `json:"user_id"`
	Action    string `json:"action"`
	Details   string `json:"details"`
	Timestamp string `json:"timestamp"`
}

// AuditResponse is the response from GET /api/admin/audit.
type AuditResponse struct {
	Entries []AuditEntry `json:"entries"`
}

// GraphEvent represents a server-sent event from the /api/events endpoint.
type GraphEvent struct {
	Event string `json:"event"`
	Data  string `json:"data"`
}

// FactsAbout returns all facts whose subject or object matches entityName
// (case-insensitive).
func (r *ContextResponse) FactsAbout(entityName string) []ContextFact {
	lower := strings.ToLower(entityName)
	var result []ContextFact
	for _, f := range r.Facts {
		if strings.ToLower(f.Subject) == lower || strings.ToLower(f.Object) == lower {
			result = append(result, f)
		}
	}
	return result
}

// IsDuplicate reports whether the remember operation wrote zero facts,
// indicating the statement was already known.
func (r *RememberResponse) IsDuplicate() bool {
	return r.FactsWritten == 0
}

// Failures returns the per-statement results that did not succeed.
func (r *BatchRememberResponse) Failures() []BatchRememberResult {
	var out []BatchRememberResult
	for _, res := range r.Results {
		if !res.OK {
			out = append(out, res)
		}
	}
	return out
}
