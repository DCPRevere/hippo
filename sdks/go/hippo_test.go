package hippo

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// helper: start a test server that records requests and replies with a canned response.
type recorded struct {
	method     string
	path       string
	requestURI string
	auth       string
	body       string
}

func testServer(t *testing.T, statusCode int, response interface{}, rec *recorded) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if rec != nil {
			rec.method = r.Method
			rec.path = r.URL.Path
			rec.requestURI = r.RequestURI
			rec.auth = r.Header.Get("Authorization")
			if r.Body != nil {
				b, _ := io.ReadAll(r.Body)
				rec.body = string(b)
			}
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(statusCode)
		if response != nil {
			json.NewEncoder(w).Encode(response)
		}
	}))
}

func TestRemember(t *testing.T) {
	want := RememberResponse{
		EntitiesCreated:          2,
		EntitiesResolved:         1,
		FactsWritten:             3,
		ContradictionsInvalidated: 0,
	}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("test-key"))
	got, err := c.Remember(context.Background(), &RememberRequest{
		Statement: "Alice knows Bob",
	})
	if err != nil {
		t.Fatal(err)
	}
	if rec.method != "POST" {
		t.Errorf("method = %s, want POST", rec.method)
	}
	if rec.path != "/remember" {
		t.Errorf("path = %s, want /remember", rec.path)
	}
	if rec.auth != "Bearer test-key" {
		t.Errorf("auth = %q, want %q", rec.auth, "Bearer test-key")
	}
	if got.EntitiesCreated != want.EntitiesCreated {
		t.Errorf("EntitiesCreated = %d, want %d", got.EntitiesCreated, want.EntitiesCreated)
	}
	if got.FactsWritten != want.FactsWritten {
		t.Errorf("FactsWritten = %d, want %d", got.FactsWritten, want.FactsWritten)
	}
}

func TestRememberBatch(t *testing.T) {
	want := BatchRememberResponse{Total: 2, Succeeded: 2, Failed: 0}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	got, err := c.RememberBatch(context.Background(), &BatchRememberRequest{
		Statements: []string{"A knows B", "B knows C"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if rec.path != "/remember/batch" {
		t.Errorf("path = %s, want /remember/batch", rec.path)
	}
	if got.Total != 2 || got.Succeeded != 2 {
		t.Errorf("got %+v, want total=2 succeeded=2", got)
	}
}

func TestContext(t *testing.T) {
	want := ContextResponse{
		Nodes: []Node{{ID: "1", Label: "Alice"}},
		Edges: []Edge{{Source: "1", Target: "2", Label: "knows"}},
	}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	got, err := c.Context(context.Background(), &ContextRequest{Query: "Alice"})
	if err != nil {
		t.Fatal(err)
	}
	if rec.path != "/context" {
		t.Errorf("path = %s, want /context", rec.path)
	}
	if len(got.Nodes) != 1 {
		t.Errorf("nodes count = %d, want 1", len(got.Nodes))
	}
	if len(got.Edges) != 1 {
		t.Errorf("edges count = %d, want 1", len(got.Edges))
	}
}

func TestAsk(t *testing.T) {
	want := AskResponse{Answer: "Yes, Alice knows Bob."}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	got, err := c.Ask(context.Background(), &AskRequest{Question: "Does Alice know Bob?"})
	if err != nil {
		t.Fatal(err)
	}
	if rec.path != "/ask" {
		t.Errorf("path = %s, want /ask", rec.path)
	}
	if got.Answer != want.Answer {
		t.Errorf("answer = %q, want %q", got.Answer, want.Answer)
	}
}

func TestHealth(t *testing.T) {
	want := HealthResponse{Status: "ok", Graph: "default"}
	srv := testServer(t, 200, want, nil)
	defer srv.Close()

	// Health should work without an API key.
	c := NewClient(srv.URL)
	got, err := c.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if got.Status != "ok" {
		t.Errorf("status = %q, want ok", got.Status)
	}
}

func TestCreateUser(t *testing.T) {
	want := CreateUserResponse{UserID: "alice", APIKey: "secret"}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("admin-key"))
	got, err := c.CreateUser(context.Background(), &CreateUserRequest{
		UserID:      "alice",
		DisplayName: "Alice",
	})
	if err != nil {
		t.Fatal(err)
	}
	if rec.path != "/admin/users" {
		t.Errorf("path = %s, want /admin/users", rec.path)
	}
	if got.APIKey != "secret" {
		t.Errorf("api_key = %q, want secret", got.APIKey)
	}
}

func TestListUsers(t *testing.T) {
	want := ListUsersResponse{Users: []User{{UserID: "alice", DisplayName: "Alice", Role: "user", KeyCount: 1}}}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	got, err := c.ListUsers(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if rec.method != "GET" {
		t.Errorf("method = %s, want GET", rec.method)
	}
	if len(got.Users) != 1 {
		t.Errorf("users count = %d, want 1", len(got.Users))
	}
}

func TestDeleteUser(t *testing.T) {
	var rec recorded
	srv := testServer(t, 204, nil, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	err := c.DeleteUser(context.Background(), "alice")
	if err != nil {
		t.Fatal(err)
	}
	if rec.method != "DELETE" {
		t.Errorf("method = %s, want DELETE", rec.method)
	}
	if rec.path != "/admin/users/alice" {
		t.Errorf("path = %s, want /admin/users/alice", rec.path)
	}
}

func TestDeleteUserPathEscape(t *testing.T) {
	var rec recorded
	srv := testServer(t, 204, nil, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	err := c.DeleteUser(context.Background(), "user/with/slashes")
	if err != nil {
		t.Fatal(err)
	}
	// The raw request URI should contain the escaped form, not literal slashes
	// in the user_id segment.
	if strings.Contains(rec.requestURI, "user/with/slashes") {
		t.Errorf("request URI was not escaped: %s", rec.requestURI)
	}
	if !strings.Contains(rec.requestURI, "user%2Fwith%2Fslashes") {
		t.Errorf("request URI missing escaped value: %s", rec.requestURI)
	}
}

func TestCreateKey(t *testing.T) {
	want := CreateKeyResponse{UserID: "alice", Label: "dev", APIKey: "newkey"}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	got, err := c.CreateKey(context.Background(), "alice", &CreateKeyRequest{Label: "dev"})
	if err != nil {
		t.Fatal(err)
	}
	if rec.path != "/admin/users/alice/keys" {
		t.Errorf("path = %s, want /admin/users/alice/keys", rec.path)
	}
	if got.APIKey != "newkey" {
		t.Errorf("api_key = %q, want newkey", got.APIKey)
	}
}

func TestListKeys(t *testing.T) {
	want := ListKeysResponse{Keys: []Key{{Label: "dev", CreatedAt: "2025-01-01T00:00:00Z"}}}
	var rec recorded
	srv := testServer(t, 200, want, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	got, err := c.ListKeys(context.Background(), "alice")
	if err != nil {
		t.Fatal(err)
	}
	if rec.method != "GET" {
		t.Errorf("method = %s, want GET", rec.method)
	}
	if len(got.Keys) != 1 {
		t.Errorf("keys count = %d, want 1", len(got.Keys))
	}
}

func TestDeleteKey(t *testing.T) {
	var rec recorded
	srv := testServer(t, 204, nil, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	err := c.DeleteKey(context.Background(), "alice", "dev")
	if err != nil {
		t.Fatal(err)
	}
	if rec.path != "/admin/users/alice/keys/dev" {
		t.Errorf("path = %s, want /admin/users/alice/keys/dev", rec.path)
	}
}

// --- Error handling tests ---

func TestErrorParsing(t *testing.T) {
	tests := []struct {
		name       string
		status     int
		body       interface{}
		wantAuth   bool
		wantForbid bool
		wantRate   bool
	}{
		{
			name:     "401 unauthorized",
			status:   401,
			body:     map[string]string{"message": "invalid token"},
			wantAuth: true,
		},
		{
			name:       "403 forbidden",
			status:     403,
			body:       map[string]string{"error": "admin only"},
			wantForbid: true,
		},
		{
			name:     "429 rate limited",
			status:   429,
			body:     map[string]string{"message": "slow down"},
			wantRate: true,
		},
		{
			name:   "500 server error",
			status: 500,
			body:   map[string]string{"message": "internal"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			srv := testServer(t, tt.status, tt.body, nil)
			defer srv.Close()

			c := NewClient(srv.URL, WithAPIKey("k"))
			_, err := c.Health(context.Background())
			if err == nil {
				t.Fatal("expected error, got nil")
			}

			he, ok := err.(*HippoError)
			if !ok {
				t.Fatalf("expected *HippoError, got %T", err)
			}
			if he.StatusCode != tt.status {
				t.Errorf("StatusCode = %d, want %d", he.StatusCode, tt.status)
			}
			if he.IsAuthError() != tt.wantAuth {
				t.Errorf("IsAuthError() = %v, want %v", he.IsAuthError(), tt.wantAuth)
			}
			if he.IsForbidden() != tt.wantForbid {
				t.Errorf("IsForbidden() = %v, want %v", he.IsForbidden(), tt.wantForbid)
			}
			if he.IsRateLimited() != tt.wantRate {
				t.Errorf("IsRateLimited() = %v, want %v", he.IsRateLimited(), tt.wantRate)
			}
		})
	}
}

func TestNoAuthHeader(t *testing.T) {
	var rec recorded
	srv := testServer(t, 200, HealthResponse{Status: "ok"}, &rec)
	defer srv.Close()

	c := NewClient(srv.URL) // no API key
	_, err := c.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if rec.auth != "" {
		t.Errorf("auth header should be empty, got %q", rec.auth)
	}
}

func TestOptionalFields(t *testing.T) {
	var rec recorded
	srv := testServer(t, 200, RememberResponse{}, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	graph := "mygraph"
	ttl := 3600
	agent := "bot"
	_, err := c.Remember(context.Background(), &RememberRequest{
		Statement:   "fact",
		SourceAgent: &agent,
		Graph:       &graph,
		TTLSecs:     &ttl,
	})
	if err != nil {
		t.Fatal(err)
	}

	var body map[string]interface{}
	json.Unmarshal([]byte(rec.body), &body)

	if body["graph"] != "mygraph" {
		t.Errorf("graph = %v, want mygraph", body["graph"])
	}
	if body["source_agent"] != "bot" {
		t.Errorf("source_agent = %v, want bot", body["source_agent"])
	}
	if body["ttl_secs"] != float64(3600) {
		t.Errorf("ttl_secs = %v, want 3600", body["ttl_secs"])
	}
}

func TestOmitsNilOptionalFields(t *testing.T) {
	var rec recorded
	srv := testServer(t, 200, RememberResponse{}, &rec)
	defer srv.Close()

	c := NewClient(srv.URL, WithAPIKey("k"))
	_, err := c.Remember(context.Background(), &RememberRequest{
		Statement: "fact",
	})
	if err != nil {
		t.Fatal(err)
	}

	if strings.Contains(rec.body, "source_agent") {
		t.Errorf("body should not contain source_agent when nil: %s", rec.body)
	}
	if strings.Contains(rec.body, "graph") {
		t.Errorf("body should not contain graph when nil: %s", rec.body)
	}
	if strings.Contains(rec.body, "ttl_secs") {
		t.Errorf("body should not contain ttl_secs when nil: %s", rec.body)
	}
}

func TestCustomHTTPClient(t *testing.T) {
	srv := testServer(t, 200, HealthResponse{Status: "ok"}, nil)
	defer srv.Close()

	custom := &http.Client{}
	c := NewClient(srv.URL, WithHTTPClient(custom))
	_, err := c.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}
}

func TestContextCancellation(t *testing.T) {
	srv := testServer(t, 200, HealthResponse{Status: "ok"}, nil)
	defer srv.Close()

	c := NewClient(srv.URL)
	ctx, cancel := context.WithCancel(context.Background())
	cancel() // cancel immediately

	_, err := c.Health(ctx)
	if err == nil {
		t.Fatal("expected error from cancelled context")
	}
}
