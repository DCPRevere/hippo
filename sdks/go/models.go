package hippo

// RememberRequest is the body for POST /remember.
type RememberRequest struct {
	Statement   string  `json:"statement"`
	SourceAgent *string `json:"source_agent,omitempty"`
	Graph       *string `json:"graph,omitempty"`
	TTLSecs     *int    `json:"ttl_secs,omitempty"`
}

// Usage contains token-usage information returned by the LLM pipeline.
type Usage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

// RememberResponse is the response from POST /remember.
type RememberResponse struct {
	EntitiesCreated          int                    `json:"entities_created"`
	EntitiesResolved         int                    `json:"entities_resolved"`
	FactsWritten             int                    `json:"facts_written"`
	ContradictionsInvalidated int                   `json:"contradictions_invalidated"`
	Usage                    map[string]interface{} `json:"usage,omitempty"`
	Trace                    map[string]interface{} `json:"trace,omitempty"`
}

// BatchRememberRequest is the body for POST /remember/batch.
type BatchRememberRequest struct {
	Statements  []string `json:"statements"`
	SourceAgent *string  `json:"source_agent,omitempty"`
	Parallel    *bool    `json:"parallel,omitempty"`
	Graph       *string  `json:"graph,omitempty"`
	TTLSecs     *int     `json:"ttl_secs,omitempty"`
}

// BatchRememberResponse is the response from POST /remember/batch.
type BatchRememberResponse struct {
	Total     int              `json:"total"`
	Succeeded int              `json:"succeeded"`
	Failed    int              `json:"failed"`
	Results   []RememberResponse `json:"results"`
}

// ContextRequest is the body for POST /context.
type ContextRequest struct {
	Query   string  `json:"query"`
	Limit   *int    `json:"limit,omitempty"`
	MaxHops *int    `json:"max_hops,omitempty"`
	Graph   *string `json:"graph,omitempty"`
}

// Node is a graph node returned in a context response.
type Node struct {
	ID         string                 `json:"id"`
	Label      string                 `json:"label"`
	Properties map[string]interface{} `json:"properties,omitempty"`
}

// Edge is a graph edge returned in a context response.
type Edge struct {
	Source     string                 `json:"source"`
	Target     string                 `json:"target"`
	Label      string                 `json:"label"`
	Properties map[string]interface{} `json:"properties,omitempty"`
}

// ContextResponse is the response from POST /context.
type ContextResponse struct {
	Nodes []Node `json:"nodes"`
	Edges []Edge `json:"edges"`
}

// AskRequest is the body for POST /ask.
type AskRequest struct {
	Question string  `json:"question"`
	Limit    *int    `json:"limit,omitempty"`
	Graph    *string `json:"graph,omitempty"`
	Verbose  *bool   `json:"verbose,omitempty"`
}

// Fact is a supporting fact returned in an ask response when verbose is true.
type Fact struct {
	Subject  string `json:"subject"`
	Relation string `json:"relation"`
	Object   string `json:"object"`
}

// AskResponse is the response from POST /ask.
type AskResponse struct {
	Answer string `json:"answer"`
	Facts  []Fact `json:"facts,omitempty"`
}

// CreateUserRequest is the body for POST /admin/users.
type CreateUserRequest struct {
	UserID      string   `json:"user_id"`
	DisplayName string   `json:"display_name"`
	Role        *string  `json:"role,omitempty"`
	Graphs      []string `json:"graphs,omitempty"`
}

// CreateUserResponse is the response from POST /admin/users.
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

// ListUsersResponse is the response from GET /admin/users.
type ListUsersResponse struct {
	Users []User `json:"users"`
}

// CreateKeyRequest is the body for POST /admin/users/{user_id}/keys.
type CreateKeyRequest struct {
	Label string `json:"label"`
}

// CreateKeyResponse is the response from POST /admin/users/{user_id}/keys.
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

// ListKeysResponse is the response from GET /admin/users/{user_id}/keys.
type ListKeysResponse struct {
	Keys []Key `json:"keys"`
}

// HealthResponse is the response from GET /health.
type HealthResponse struct {
	Status string `json:"status"`
	Graph  string `json:"graph"`
}
