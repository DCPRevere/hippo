package hippo

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
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
		EntitiesCreated:           2,
		EntitiesResolved:          1,
		FactsWritten:              3,
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

			c := NewClient(srv.URL, WithAPIKey("k"), WithMaxRetries(0))
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

// --- Retry tests ---

func TestRetryOn502ThenSuccess(t *testing.T) {
	var calls int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&calls, 1)
		if n <= 2 {
			w.WriteHeader(502)
			w.Write([]byte(`{"message":"bad gateway"}`))
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(HealthResponse{Status: "ok"})
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(3))
	got, err := c.Health(context.Background())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got.Status != "ok" {
		t.Errorf("status = %q, want ok", got.Status)
	}
	if n := atomic.LoadInt32(&calls); n != 3 {
		t.Errorf("calls = %d, want 3", n)
	}
}

func TestRetryAfterHeaderSeconds(t *testing.T) {
	var calls int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&calls, 1)
		if n == 1 {
			w.Header().Set("Retry-After", "1")
			w.WriteHeader(429)
			w.Write([]byte(`{"message":"rate limited"}`))
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(HealthResponse{Status: "ok"})
	}))
	defer srv.Close()

	start := time.Now()
	c := NewClient(srv.URL, WithMaxRetries(3))
	_, err := c.Health(context.Background())
	elapsed := time.Since(start)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	// Should have waited at least ~1 second for the Retry-After.
	if elapsed < 900*time.Millisecond {
		t.Errorf("expected at least ~1s delay for Retry-After, got %v", elapsed)
	}
	if n := atomic.LoadInt32(&calls); n != 2 {
		t.Errorf("calls = %d, want 2", n)
	}
}

func TestRetryAfterHeaderHTTPDate(t *testing.T) {
	var calls int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&calls, 1)
		if n == 1 {
			// Add 2 seconds to avoid sub-second rounding issues with RFC1123 (second granularity).
			retryAt := time.Now().Add(2 * time.Second).UTC().Format(time.RFC1123)
			w.Header().Set("Retry-After", retryAt)
			w.WriteHeader(503)
			w.Write([]byte(`{"message":"unavailable"}`))
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(HealthResponse{Status: "ok"})
	}))
	defer srv.Close()

	start := time.Now()
	c := NewClient(srv.URL, WithMaxRetries(3))
	_, err := c.Health(context.Background())
	elapsed := time.Since(start)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	// RFC1123 has second granularity, so with 2s offset we expect at least 1s of actual delay.
	if elapsed < 1*time.Second {
		t.Errorf("expected at least ~1s delay for Retry-After HTTP date, got %v", elapsed)
	}
}

func TestMaxRetriesZeroDisablesRetry(t *testing.T) {
	var calls int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&calls, 1)
		w.WriteHeader(502)
		w.Write([]byte(`{"message":"bad gateway"}`))
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(0))
	_, err := c.Health(context.Background())
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	he, ok := err.(*HippoError)
	if !ok {
		t.Fatalf("expected *HippoError, got %T", err)
	}
	if he.StatusCode != 502 {
		t.Errorf("StatusCode = %d, want 502", he.StatusCode)
	}
	if n := atomic.LoadInt32(&calls); n != 1 {
		t.Errorf("calls = %d, want 1 (no retries)", n)
	}
}

func TestRetryCancelledDuringWait(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Retry-After", "60")
		w.WriteHeader(503)
		w.Write([]byte(`{"message":"unavailable"}`))
	}))
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()

	c := NewClient(srv.URL, WithMaxRetries(5), WithTimeout(10*time.Second))
	_, err := c.Health(ctx)
	if err == nil {
		t.Fatal("expected error from cancelled context")
	}
}

// --- Timeout tests ---

func TestWithTimeout(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		time.Sleep(2 * time.Second)
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(HealthResponse{Status: "ok"})
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithTimeout(200*time.Millisecond), WithMaxRetries(0))
	_, err := c.Health(context.Background())
	if err == nil {
		t.Fatal("expected timeout error, got nil")
	}
}

func TestWithTimeoutNotAppliedWhenDeadlineSet(t *testing.T) {
	srv := testServer(t, 200, HealthResponse{Status: "ok"}, nil)
	defer srv.Close()

	// Set a very short client timeout, but use a context with a generous deadline.
	c := NewClient(srv.URL, WithTimeout(1*time.Millisecond))
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	got, err := c.Health(ctx)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got.Status != "ok" {
		t.Errorf("status = %q, want ok", got.Status)
	}
}

// --- Env var config tests ---

func TestNewClientEnvURL(t *testing.T) {
	t.Setenv("HIPPO_URL", "http://env-host:9999")
	c := NewClient("")
	if c.baseURL != "http://env-host:9999" {
		t.Errorf("baseURL = %q, want http://env-host:9999", c.baseURL)
	}
}

func TestNewClientDefaultURL(t *testing.T) {
	t.Setenv("HIPPO_URL", "")
	c := NewClient("")
	if c.baseURL != defaultBaseURL {
		t.Errorf("baseURL = %q, want %q", c.baseURL, defaultBaseURL)
	}
}

func TestWithAPIKeyFromEnv(t *testing.T) {
	t.Setenv("HIPPO_API_KEY", "env-secret")
	c := NewClient("http://localhost", WithAPIKeyFromEnv())
	if c.apiKey != "env-secret" {
		t.Errorf("apiKey = %q, want env-secret", c.apiKey)
	}
}

func TestWithAPIKeyFromEnvEmpty(t *testing.T) {
	t.Setenv("HIPPO_API_KEY", "")
	c := NewClient("http://localhost", WithAPIKeyFromEnv())
	if c.apiKey != "" {
		t.Errorf("apiKey = %q, want empty", c.apiKey)
	}
}

func TestExplicitAPIKeyOverridesEnv(t *testing.T) {
	t.Setenv("HIPPO_API_KEY", "env-secret")
	c := NewClient("http://localhost", WithAPIKeyFromEnv(), WithAPIKey("explicit"))
	if c.apiKey != "explicit" {
		t.Errorf("apiKey = %q, want explicit", c.apiKey)
	}
}

// --- Response helper tests ---

func TestFindNode(t *testing.T) {
	resp := &ContextResponse{
		Nodes: []Node{
			{ID: "1", Label: "Alice"},
			{ID: "2", Label: "Bob"},
		},
	}

	if n := resp.FindNode("alice"); n == nil || n.ID != "1" {
		t.Errorf("FindNode(alice) = %v, want node with ID 1", n)
	}
	if n := resp.FindNode("Bob"); n == nil || n.ID != "2" {
		t.Errorf("FindNode(Bob) = %v, want node with ID 2", n)
	}
	if n := resp.FindNode("Charlie"); n != nil {
		t.Errorf("FindNode(Charlie) = %v, want nil", n)
	}
}

func TestFactsAbout(t *testing.T) {
	resp := &ContextResponse{
		Edges: []Edge{
			{Source: "Alice", Target: "Bob", Label: "knows"},
			{Source: "Bob", Target: "Charlie", Label: "likes"},
			{Source: "Dave", Target: "Eve", Label: "married"},
		},
	}

	facts := resp.FactsAbout("bob")
	if len(facts) != 2 {
		t.Errorf("FactsAbout(bob) returned %d edges, want 2", len(facts))
	}

	facts = resp.FactsAbout("Dave")
	if len(facts) != 1 {
		t.Errorf("FactsAbout(Dave) returned %d edges, want 1", len(facts))
	}

	facts = resp.FactsAbout("nobody")
	if len(facts) != 0 {
		t.Errorf("FactsAbout(nobody) returned %d edges, want 0", len(facts))
	}
}

func TestIsDuplicate(t *testing.T) {
	dup := &RememberResponse{FactsWritten: 0}
	if !dup.IsDuplicate() {
		t.Error("IsDuplicate() = false, want true when FactsWritten == 0")
	}

	notDup := &RememberResponse{FactsWritten: 2}
	if notDup.IsDuplicate() {
		t.Error("IsDuplicate() = true, want false when FactsWritten > 0")
	}
}

func TestBatchFailures(t *testing.T) {
	resp := &BatchRememberResponse{
		Total:     3,
		Succeeded: 2,
		Failed:    1,
		Results: []RememberResponse{
			{FactsWritten: 2},
			{FactsWritten: 0},
			{FactsWritten: 1},
		},
	}

	failures := resp.Failures()
	if len(failures) != 1 {
		t.Errorf("Failures() returned %d, want 1", len(failures))
	}
	if failures[0].FactsWritten != 0 {
		t.Errorf("failure FactsWritten = %d, want 0", failures[0].FactsWritten)
	}
}

func TestBatchNoFailures(t *testing.T) {
	resp := &BatchRememberResponse{
		Results: []RememberResponse{
			{FactsWritten: 1},
			{FactsWritten: 2},
		},
	}
	if f := resp.Failures(); len(f) != 0 {
		t.Errorf("Failures() returned %d, want 0", len(f))
	}
}

// --- SSE Events tests ---

func TestEventsStream(t *testing.T) {
	sseBody := "event: fact_created\ndata: {\"id\":1}\n\nevent: entity_resolved\ndata: {\"id\":2}\n\n"
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/events" {
			t.Errorf("path = %s, want /events", r.URL.Path)
		}
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(200)
		w.Write([]byte(sseBody))
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(0))
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	ch, err := c.Events(ctx)
	if err != nil {
		t.Fatalf("Events() error: %v", err)
	}

	var events []GraphEvent
	for ev := range ch {
		events = append(events, ev)
	}

	if len(events) != 2 {
		t.Fatalf("received %d events, want 2", len(events))
	}
	if events[0].Event != "fact_created" {
		t.Errorf("events[0].Event = %q, want fact_created", events[0].Event)
	}
	if events[0].Data != `{"id":1}` {
		t.Errorf("events[0].Data = %q, want {\"id\":1}", events[0].Data)
	}
	if events[1].Event != "entity_resolved" {
		t.Errorf("events[1].Event = %q, want entity_resolved", events[1].Event)
	}
}

func TestEventsWithGraph(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if g := r.URL.Query().Get("graph"); g != "mygraph" {
			t.Errorf("graph param = %q, want mygraph", g)
		}
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(200)
		fmt.Fprint(w, "event: ping\ndata: ok\n\n")
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(0))
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	ch, err := c.Events(ctx, WithGraph("mygraph"))
	if err != nil {
		t.Fatalf("Events() error: %v", err)
	}

	ev := <-ch
	if ev.Event != "ping" {
		t.Errorf("event = %q, want ping", ev.Event)
	}
}

func TestEventsContextCancel(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(200)
		flusher, ok := w.(http.Flusher)
		if ok {
			flusher.Flush()
		}
		// Keep connection open until client disconnects.
		<-r.Context().Done()
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(0))
	ctx, cancel := context.WithCancel(context.Background())

	ch, err := c.Events(ctx)
	if err != nil {
		t.Fatalf("Events() error: %v", err)
	}

	cancel()

	// Channel should close after context cancellation.
	select {
	case _, ok := <-ch:
		if ok {
			t.Error("expected channel to be closed")
		}
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for channel to close")
	}
}

func TestEventsMultilineData(t *testing.T) {
	sseBody := "event: multi\ndata: line1\ndata: line2\n\n"
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(200)
		w.Write([]byte(sseBody))
	}))
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(0))
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	ch, err := c.Events(ctx)
	if err != nil {
		t.Fatal(err)
	}

	ev := <-ch
	if ev.Data != "line1\nline2" {
		t.Errorf("data = %q, want \"line1\\nline2\"", ev.Data)
	}
}

func TestEventsHTTPError(t *testing.T) {
	srv := testServer(t, 401, map[string]string{"message": "unauthorized"}, nil)
	defer srv.Close()

	c := NewClient(srv.URL, WithMaxRetries(0))
	_, err := c.Events(context.Background())
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	he, ok := err.(*HippoError)
	if !ok {
		t.Fatalf("expected *HippoError, got %T", err)
	}
	if he.StatusCode != 401 {
		t.Errorf("StatusCode = %d, want 401", he.StatusCode)
	}
}

// --- Logger tests ---

type testLogger struct {
	debugMsgs []string
	warnMsgs  []string
}

func (l *testLogger) Debug(msg string, args ...any) {
	l.debugMsgs = append(l.debugMsgs, fmt.Sprintf(msg, args...))
}

func (l *testLogger) Warn(msg string, args ...any) {
	l.warnMsgs = append(l.warnMsgs, fmt.Sprintf(msg, args...))
}

func TestLoggerDebugOnSuccess(t *testing.T) {
	srv := testServer(t, 200, HealthResponse{Status: "ok"}, nil)
	defer srv.Close()

	lg := &testLogger{}
	c := NewClient(srv.URL, WithLogger(lg))
	_, err := c.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	if len(lg.debugMsgs) < 2 {
		t.Errorf("expected at least 2 debug messages (request + response), got %d: %v", len(lg.debugMsgs), lg.debugMsgs)
	}
}

func TestLoggerWarnOnRetry(t *testing.T) {
	var calls int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&calls, 1)
		if n == 1 {
			w.WriteHeader(503)
			w.Write([]byte(`{"message":"unavailable"}`))
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(HealthResponse{Status: "ok"})
	}))
	defer srv.Close()

	lg := &testLogger{}
	c := NewClient(srv.URL, WithLogger(lg), WithMaxRetries(3))
	_, err := c.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	if len(lg.warnMsgs) < 1 {
		t.Errorf("expected at least 1 warn message for retry, got %d", len(lg.warnMsgs))
	}
}

func TestLoggerNilByDefault(t *testing.T) {
	srv := testServer(t, 200, HealthResponse{Status: "ok"}, nil)
	defer srv.Close()

	c := NewClient(srv.URL)
	if c.logger != nil {
		t.Error("expected nil logger by default")
	}
	// Should not panic with nil logger.
	_, err := c.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}
}
