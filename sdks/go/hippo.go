// Package hippo provides a Go client for the Hippo natural-language database REST API.
package hippo

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"math/rand"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

const (
	defaultBaseURL    = "http://localhost:3000"
	defaultTimeout    = 30 * time.Second
	defaultMaxRetries = 3
)

// Logger is the interface for optional structured logging.
type Logger interface {
	Debug(msg string, args ...any)
	Warn(msg string, args ...any)
}

// Client is a Hippo API client. It is safe for concurrent use.
type Client struct {
	baseURL    string
	apiKey     string
	httpClient *http.Client
	timeout    time.Duration
	maxRetries int
	logger     Logger
}

// Option configures a Client.
type Option func(*Client)

// WithAPIKey sets the Bearer token used for authenticated endpoints.
func WithAPIKey(key string) Option {
	return func(c *Client) {
		c.apiKey = key
	}
}

// WithAPIKeyFromEnv reads the API key from the HIPPO_API_KEY environment variable.
func WithAPIKeyFromEnv() Option {
	return func(c *Client) {
		if key := os.Getenv("HIPPO_API_KEY"); key != "" {
			c.apiKey = key
		}
	}
}

// WithHTTPClient sets a custom http.Client for the Hippo client.
func WithHTTPClient(hc *http.Client) Option {
	return func(c *Client) {
		c.httpClient = hc
	}
}

// WithTimeout sets the default request timeout. If the caller's context
// already has a deadline, this timeout is not applied.
func WithTimeout(d time.Duration) Option {
	return func(c *Client) {
		c.timeout = d
	}
}

// WithMaxRetries sets the maximum number of retry attempts for retryable
// status codes (429, 502, 503, 504). Set to 0 to disable retries.
func WithMaxRetries(n int) Option {
	return func(c *Client) {
		c.maxRetries = n
	}
}

// WithLogger sets a logger for debug and warning messages.
func WithLogger(l Logger) Option {
	return func(c *Client) {
		c.logger = l
	}
}

// NewClient creates a new Hippo API client.
// baseURL is the root URL of the Hippo server (e.g. "http://localhost:3000").
// If baseURL is empty, it falls back to the HIPPO_URL environment variable,
// then to the default "http://localhost:3000".
func NewClient(baseURL string, opts ...Option) *Client {
	if baseURL == "" {
		baseURL = os.Getenv("HIPPO_URL")
	}
	if baseURL == "" {
		baseURL = defaultBaseURL
	}
	c := &Client{
		baseURL:    strings.TrimRight(baseURL, "/"),
		httpClient: http.DefaultClient,
		timeout:    defaultTimeout,
		maxRetries: defaultMaxRetries,
	}
	for _, o := range opts {
		o(c)
	}
	return c
}

func (c *Client) logDebug(msg string, args ...any) {
	if c.logger != nil {
		c.logger.Debug(msg, args...)
	}
}

func (c *Client) logWarn(msg string, args ...any) {
	if c.logger != nil {
		c.logger.Warn(msg, args...)
	}
}

// apiPath prepends "/api" to all paths except "/health", which is the only
// route the server mounts at the root.
func apiPath(path string) string {
	if path == "/health" || strings.HasPrefix(path, "/api/") || path == "/api" {
		return path
	}
	return "/api" + path
}

func (c *Client) newRequest(ctx context.Context, method, path string, body interface{}) (*http.Request, error) {
	u := c.baseURL + apiPath(path)

	var bodyReader io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("hippo: marshal request: %w", err)
		}
		bodyReader = bytes.NewReader(b)
	}

	req, err := http.NewRequestWithContext(ctx, method, u, bodyReader)
	if err != nil {
		return nil, err
	}

	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if c.apiKey != "" {
		req.Header.Set("Authorization", "Bearer "+c.apiKey)
	}
	return req, nil
}

// contextWithTimeout wraps ctx with the client's default timeout if ctx
// does not already have a deadline set.
func (c *Client) contextWithTimeout(ctx context.Context) (context.Context, context.CancelFunc) {
	if _, ok := ctx.Deadline(); ok {
		return ctx, func() {}
	}
	return context.WithTimeout(ctx, c.timeout)
}

// isRetryable returns true for status codes that should be retried.
func isRetryable(code int) bool {
	return code == 429 || code == 502 || code == 503 || code == 504
}

// parseRetryAfter parses a Retry-After header value, which may be seconds
// or an HTTP date. Returns 0 if the header is empty or unparseable.
func parseRetryAfter(val string) time.Duration {
	if val == "" {
		return 0
	}
	// Try seconds first.
	if secs, err := strconv.Atoi(val); err == nil && secs > 0 {
		return time.Duration(secs) * time.Second
	}
	// Try HTTP date format.
	if t, err := time.Parse(time.RFC1123, val); err == nil {
		d := time.Until(t)
		if d > 0 {
			return d
		}
	}
	return 0
}

func (c *Client) do(req *http.Request, out interface{}) error {
	ctx := req.Context()

	// We may need to re-send the request body on retries.
	// Save it so we can reconstruct the request.
	var bodyBytes []byte
	if req.Body != nil {
		var err error
		bodyBytes, err = io.ReadAll(req.Body)
		if err != nil {
			return fmt.Errorf("hippo: read request body: %w", err)
		}
		req.Body = io.NopCloser(bytes.NewReader(bodyBytes))
	}

	c.logDebug("request", "method", req.Method, "url", req.URL.String())

	for attempt := 0; ; attempt++ {
		// Reset body for retries.
		if attempt > 0 && bodyBytes != nil {
			req.Body = io.NopCloser(bytes.NewReader(bodyBytes))
		}

		resp, err := c.httpClient.Do(req)
		if err != nil {
			return err
		}

		data, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			return fmt.Errorf("hippo: read response: %w", err)
		}

		c.logDebug("response", "status", resp.StatusCode, "attempt", attempt+1)

		if isRetryable(resp.StatusCode) && attempt < c.maxRetries {
			// Determine backoff duration.
			backoff := parseRetryAfter(resp.Header.Get("Retry-After"))
			if backoff == 0 {
				// Exponential backoff: 500ms, 1s, 2s, 4s...
				base := time.Duration(500) * time.Millisecond
				backoff = base * (1 << uint(attempt))
				// Add jitter: +/- 25%.
				jitter := time.Duration(rand.Int63n(int64(backoff) / 2))
				backoff = backoff - backoff/4 + jitter
			}

			c.logWarn("retrying", "status", resp.StatusCode, "attempt", attempt+1, "backoff", backoff)

			timer := time.NewTimer(backoff)
			select {
			case <-ctx.Done():
				timer.Stop()
				return ctx.Err()
			case <-timer.C:
			}
			continue
		}

		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			msg := string(data)
			var errBody struct {
				Message string `json:"message"`
				Error   string `json:"error"`
			}
			if json.Unmarshal(data, &errBody) == nil {
				if errBody.Message != "" {
					msg = errBody.Message
				} else if errBody.Error != "" {
					msg = errBody.Error
				}
			}
			return &HippoError{StatusCode: resp.StatusCode, Message: msg}
		}

		if out != nil {
			if err := json.Unmarshal(data, out); err != nil {
				return fmt.Errorf("hippo: decode response: %w", err)
			}
		}
		return nil
	}
}

// doRequest is a convenience that creates a request, applies the client timeout,
// and executes it with retry logic.
func (c *Client) doRequest(ctx context.Context, method, path string, body, out interface{}) error {
	ctx, cancel := c.contextWithTimeout(ctx)
	defer cancel()

	req, err := c.newRequest(ctx, method, path, body)
	if err != nil {
		return err
	}
	return c.do(req, out)
}

// Remember stores a natural-language statement in the graph.
func (c *Client) Remember(ctx context.Context, req *RememberRequest) (*RememberResponse, error) {
	var out RememberResponse
	if err := c.doRequest(ctx, http.MethodPost, "/remember", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// RememberBatch stores multiple statements in one call.
func (c *Client) RememberBatch(ctx context.Context, req *BatchRememberRequest) (*BatchRememberResponse, error) {
	var out BatchRememberResponse
	if err := c.doRequest(ctx, http.MethodPost, "/remember/batch", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Context retrieves graph context relevant to a natural-language query.
func (c *Client) Context(ctx context.Context, req *ContextRequest) (*ContextResponse, error) {
	var out ContextResponse
	if err := c.doRequest(ctx, http.MethodPost, "/context", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Ask poses a natural-language question and receives a synthesized answer.
func (c *Client) Ask(ctx context.Context, req *AskRequest) (*AskResponse, error) {
	var out AskResponse
	if err := c.doRequest(ctx, http.MethodPost, "/ask", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// CreateUser creates a new user (admin only).
func (c *Client) CreateUser(ctx context.Context, req *CreateUserRequest) (*CreateUserResponse, error) {
	var out CreateUserResponse
	if err := c.doRequest(ctx, http.MethodPost, "/admin/users", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// ListUsers returns all users (admin only).
func (c *Client) ListUsers(ctx context.Context) (*ListUsersResponse, error) {
	var out ListUsersResponse
	if err := c.doRequest(ctx, http.MethodGet, "/admin/users", nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// DeleteUser removes a user by ID (admin only).
func (c *Client) DeleteUser(ctx context.Context, userID string) error {
	path := "/admin/users/" + url.PathEscape(userID)
	return c.doRequest(ctx, http.MethodDelete, path, nil, nil)
}

// CreateKey creates a new API key for a user (admin only).
func (c *Client) CreateKey(ctx context.Context, userID string, req *CreateKeyRequest) (*CreateKeyResponse, error) {
	path := "/admin/users/" + url.PathEscape(userID) + "/keys"
	var out CreateKeyResponse
	if err := c.doRequest(ctx, http.MethodPost, path, req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// ListKeys returns all API keys for a user (admin only).
func (c *Client) ListKeys(ctx context.Context, userID string) (*ListKeysResponse, error) {
	path := "/admin/users/" + url.PathEscape(userID) + "/keys"
	var out ListKeysResponse
	if err := c.doRequest(ctx, http.MethodGet, path, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// DeleteKey removes an API key by label for a user (admin only).
func (c *Client) DeleteKey(ctx context.Context, userID, label string) error {
	path := "/admin/users/" + url.PathEscape(userID) + "/keys/" + url.PathEscape(label)
	return c.doRequest(ctx, http.MethodDelete, path, nil, nil)
}

// Health checks server health. This endpoint does not require authentication.
func (c *Client) Health(ctx context.Context) (*HealthResponse, error) {
	var out HealthResponse
	if err := c.doRequest(ctx, http.MethodGet, "/health", nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// --- REST resources ---

func entityPath(id string, graph *string) string {
	p := "/entities/" + url.PathEscape(id)
	if graph != nil {
		p += "?graph=" + url.QueryEscape(*graph)
	}
	return p
}

func edgePath(id int64, suffix string, graph *string) string {
	p := fmt.Sprintf("/edges/%d", id)
	if suffix != "" {
		p += "/" + suffix
	}
	if graph != nil {
		p += "?graph=" + url.QueryEscape(*graph)
	}
	return p
}

// GetEntity fetches a single entity by ID.
func (c *Client) GetEntity(ctx context.Context, id string, graph *string) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodGet, entityPath(id, graph), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// DeleteEntity removes an entity by ID and invalidates all its edges.
func (c *Client) DeleteEntity(ctx context.Context, id string, graph *string) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodDelete, entityPath(id, graph), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// EntityEdges returns all active edges originating from an entity.
func (c *Client) EntityEdges(ctx context.Context, id string, graph *string) ([]map[string]any, error) {
	var out []map[string]any
	p := "/entities/" + url.PathEscape(id) + "/edges"
	if graph != nil {
		p += "?graph=" + url.QueryEscape(*graph)
	}
	if err := c.doRequest(ctx, http.MethodGet, p, nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// GetEdge fetches a single edge by ID.
func (c *Client) GetEdge(ctx context.Context, id int64, graph *string) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodGet, edgePath(id, "", graph), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// EdgeProvenance returns the supersession history for an edge.
func (c *Client) EdgeProvenance(ctx context.Context, id int64, graph *string) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodGet, edgePath(id, "provenance", graph), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// --- Destructive operations ---

// Retract explicitly retracts a fact.
func (c *Client) Retract(ctx context.Context, req *RetractRequest) (*RetractResponse, error) {
	var out RetractResponse
	if err := c.doRequest(ctx, http.MethodPost, "/retract", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Correct retracts an old fact and observes a new one in a single call.
func (c *Client) Correct(ctx context.Context, req *CorrectRequest) (*CorrectResponse, error) {
	var out CorrectResponse
	if err := c.doRequest(ctx, http.MethodPost, "/correct", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// --- Operations ---

// Maintain runs a single maintenance/Dreamer cycle.
func (c *Client) Maintain(ctx context.Context) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodPost, "/maintain", struct{}{}, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Graph returns the full graph as JSON.
func (c *Client) Graph(ctx context.Context, graph *string) (map[string]any, error) {
	p := "/graph"
	if graph != nil {
		p += "?graph=" + url.QueryEscape(*graph)
	}
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodGet, p, nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// GraphExport returns the graph in the requested format ("graphml" or "csv")
// as a raw byte slice.
func (c *Client) GraphExport(ctx context.Context, graph *string, format string) ([]byte, error) {
	q := url.Values{}
	q.Set("format", format)
	if graph != nil {
		q.Set("graph", *graph)
	}
	p := "/graph?" + q.Encode()
	return c.getRaw(ctx, p)
}

// Metrics returns Prometheus metrics as raw text.
func (c *Client) Metrics(ctx context.Context) ([]byte, error) {
	return c.getRaw(ctx, "/metrics")
}

// OpenAPI returns the OpenAPI YAML document as raw text.
func (c *Client) OpenAPI(ctx context.Context) ([]byte, error) {
	return c.getRaw(ctx, "/openapi.yaml")
}

// --- Graphs ---

// ListGraphs returns the registered graph names and the default graph.
func (c *Client) ListGraphs(ctx context.Context) (*GraphsListResponse, error) {
	var out GraphsListResponse
	if err := c.doRequest(ctx, http.MethodGet, "/graphs", nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// DropGraph deletes a graph (admin only).
func (c *Client) DropGraph(ctx context.Context, name string) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodDelete, "/graphs/drop/"+url.PathEscape(name), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Seed inserts entities and edges directly. Body shape matches the server's
// AdminSeedRequest.
func (c *Client) Seed(ctx context.Context, body any) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodPost, "/seed", body, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Backup downloads the JSON backup payload for a graph as raw bytes.
func (c *Client) Backup(ctx context.Context, graph *string) ([]byte, error) {
	body := map[string]any{}
	if graph != nil {
		body["graph"] = *graph
	}
	ctx, cancel := c.contextWithTimeout(ctx)
	defer cancel()
	req, err := c.newRequest(ctx, http.MethodPost, "/admin/backup", body)
	if err != nil {
		return nil, err
	}
	return c.doRaw(req)
}

// Restore restores a backup payload into the target graph.
func (c *Client) Restore(ctx context.Context, body any) (map[string]any, error) {
	var out map[string]any
	if err := c.doRequest(ctx, http.MethodPost, "/admin/restore", body, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Audit returns recent audit log entries (admin only).
func (c *Client) Audit(ctx context.Context, userID, action *string, limit *int) (*AuditResponse, error) {
	q := url.Values{}
	if userID != nil {
		q.Set("user_id", *userID)
	}
	if action != nil {
		q.Set("action", *action)
	}
	if limit != nil {
		q.Set("limit", strconv.Itoa(*limit))
	}
	p := "/admin/audit"
	if len(q) > 0 {
		p += "?" + q.Encode()
	}
	var out AuditResponse
	if err := c.doRequest(ctx, http.MethodGet, p, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// getRaw performs a GET and returns the raw response body. It applies the
// client's timeout but does not retry — callers that need retries should fall
// through doRequest with a typed body.
func (c *Client) getRaw(ctx context.Context, path string) ([]byte, error) {
	ctx, cancel := c.contextWithTimeout(ctx)
	defer cancel()
	req, err := c.newRequest(ctx, http.MethodGet, path, nil)
	if err != nil {
		return nil, err
	}
	return c.doRaw(req)
}

func (c *Client) doRaw(req *http.Request) ([]byte, error) {
	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("hippo: read response: %w", err)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, &HippoError{StatusCode: resp.StatusCode, Message: string(data)}
	}
	return data, nil
}

// EventsOption configures the Events stream.
type EventsOption func(url.Values)

// WithGraph filters the event stream to a specific graph.
func WithGraph(name string) EventsOption {
	return func(v url.Values) {
		v.Set("graph", name)
	}
}

// Events opens an SSE connection to the /events endpoint and returns a channel
// of GraphEvent. The channel is closed when the context is cancelled or the
// connection drops.
func (c *Client) Events(ctx context.Context, opts ...EventsOption) (<-chan GraphEvent, error) {
	params := url.Values{}
	for _, o := range opts {
		o(params)
	}

	path := "/events"
	if len(params) > 0 {
		path += "?" + params.Encode()
	}

	req, err := c.newRequest(ctx, http.MethodGet, path, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "text/event-stream")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, err
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		data, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		msg := string(data)
		var errBody struct {
			Message string `json:"message"`
			Error   string `json:"error"`
		}
		if json.Unmarshal(data, &errBody) == nil {
			if errBody.Message != "" {
				msg = errBody.Message
			} else if errBody.Error != "" {
				msg = errBody.Error
			}
		}
		return nil, &HippoError{StatusCode: resp.StatusCode, Message: msg}
	}

	ch := make(chan GraphEvent)
	go func() {
		defer close(ch)
		defer resp.Body.Close()

		scanner := bufio.NewScanner(resp.Body)
		var event, data string
		for scanner.Scan() {
			line := scanner.Text()

			if line == "" {
				// Empty line = end of event.
				if data != "" {
					ge := GraphEvent{Event: event, Data: data}
					select {
					case ch <- ge:
					case <-ctx.Done():
						return
					}
				}
				event = ""
				data = ""
				continue
			}

			if strings.HasPrefix(line, "event:") {
				event = strings.TrimSpace(strings.TrimPrefix(line, "event:"))
			} else if strings.HasPrefix(line, "data:") {
				d := strings.TrimSpace(strings.TrimPrefix(line, "data:"))
				if data != "" {
					data += "\n" + d
				} else {
					data = d
				}
			}
		}
		// Flush any remaining event.
		if data != "" {
			select {
			case ch <- GraphEvent{Event: event, Data: data}:
			case <-ctx.Done():
			}
		}
	}()

	return ch, nil
}
