// Package hippo provides a Go client for the Hippo natural-language database REST API.
package hippo

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
)

// Client is a Hippo API client. It is safe for concurrent use.
type Client struct {
	baseURL    string
	apiKey     string
	httpClient *http.Client
}

// Option configures a Client.
type Option func(*Client)

// WithAPIKey sets the Bearer token used for authenticated endpoints.
func WithAPIKey(key string) Option {
	return func(c *Client) {
		c.apiKey = key
	}
}

// WithHTTPClient sets a custom http.Client for the Hippo client.
func WithHTTPClient(hc *http.Client) Option {
	return func(c *Client) {
		c.httpClient = hc
	}
}

// NewClient creates a new Hippo API client.
// baseURL is the root URL of the Hippo server (e.g. "http://localhost:3000").
func NewClient(baseURL string, opts ...Option) *Client {
	c := &Client{
		baseURL:    strings.TrimRight(baseURL, "/"),
		httpClient: http.DefaultClient,
	}
	for _, o := range opts {
		o(c)
	}
	return c
}

func (c *Client) newRequest(ctx context.Context, method, path string, body interface{}) (*http.Request, error) {
	u := c.baseURL + path

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

func (c *Client) do(req *http.Request, out interface{}) error {
	resp, err := c.httpClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("hippo: read response: %w", err)
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		msg := string(data)
		// Try to extract a message field from JSON error bodies.
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

// Remember stores a natural-language statement in the graph.
func (c *Client) Remember(ctx context.Context, req *RememberRequest) (*RememberResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodPost, "/remember", req)
	if err != nil {
		return nil, err
	}
	var out RememberResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// RememberBatch stores multiple statements in one call.
func (c *Client) RememberBatch(ctx context.Context, req *BatchRememberRequest) (*BatchRememberResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodPost, "/remember/batch", req)
	if err != nil {
		return nil, err
	}
	var out BatchRememberResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Context retrieves graph context relevant to a natural-language query.
func (c *Client) Context(ctx context.Context, req *ContextRequest) (*ContextResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodPost, "/context", req)
	if err != nil {
		return nil, err
	}
	var out ContextResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Ask poses a natural-language question and receives a synthesized answer.
func (c *Client) Ask(ctx context.Context, req *AskRequest) (*AskResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodPost, "/ask", req)
	if err != nil {
		return nil, err
	}
	var out AskResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// CreateUser creates a new user (admin only).
func (c *Client) CreateUser(ctx context.Context, req *CreateUserRequest) (*CreateUserResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodPost, "/admin/users", req)
	if err != nil {
		return nil, err
	}
	var out CreateUserResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// ListUsers returns all users (admin only).
func (c *Client) ListUsers(ctx context.Context) (*ListUsersResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodGet, "/admin/users", nil)
	if err != nil {
		return nil, err
	}
	var out ListUsersResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// DeleteUser removes a user by ID (admin only).
func (c *Client) DeleteUser(ctx context.Context, userID string) error {
	path := "/admin/users/" + url.PathEscape(userID)
	httpReq, err := c.newRequest(ctx, http.MethodDelete, path, nil)
	if err != nil {
		return err
	}
	return c.do(httpReq, nil)
}

// CreateKey creates a new API key for a user (admin only).
func (c *Client) CreateKey(ctx context.Context, userID string, req *CreateKeyRequest) (*CreateKeyResponse, error) {
	path := "/admin/users/" + url.PathEscape(userID) + "/keys"
	httpReq, err := c.newRequest(ctx, http.MethodPost, path, req)
	if err != nil {
		return nil, err
	}
	var out CreateKeyResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// ListKeys returns all API keys for a user (admin only).
func (c *Client) ListKeys(ctx context.Context, userID string) (*ListKeysResponse, error) {
	path := "/admin/users/" + url.PathEscape(userID) + "/keys"
	httpReq, err := c.newRequest(ctx, http.MethodGet, path, nil)
	if err != nil {
		return nil, err
	}
	var out ListKeysResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// DeleteKey removes an API key by label for a user (admin only).
func (c *Client) DeleteKey(ctx context.Context, userID, label string) error {
	path := "/admin/users/" + url.PathEscape(userID) + "/keys/" + url.PathEscape(label)
	httpReq, err := c.newRequest(ctx, http.MethodDelete, path, nil)
	if err != nil {
		return err
	}
	return c.do(httpReq, nil)
}

// Health checks server health. This endpoint does not require authentication.
func (c *Client) Health(ctx context.Context) (*HealthResponse, error) {
	httpReq, err := c.newRequest(ctx, http.MethodGet, "/health", nil)
	if err != nil {
		return nil, err
	}
	var out HealthResponse
	if err := c.do(httpReq, &out); err != nil {
		return nil, err
	}
	return &out, nil
}
