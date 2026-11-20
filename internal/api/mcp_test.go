package api

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"stock-predictor/internal/apikeys"
	"stock-predictor/internal/config"
	"stock-predictor/internal/models"
	"stock-predictor/internal/trades"
)

// bearerTransport puts the key on every request, which is what a real client does with its header.
type bearerTransport struct{ token string }

func (b bearerTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	clone := r.Clone(r.Context())
	clone.Header.Set("Authorization", "Bearer "+b.token)
	return http.DefaultTransport.RoundTrip(clone)
}

func mcpSession(t *testing.T, scope apikeys.Scope) (*mcp.ClientSession, func()) {
	t.Helper()
	pool, engine, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	token, _, err := apikeys.New(pool, "test client", scope, nil, time.Now().UTC())
	if err != nil {
		t.Fatal(err)
	}
	st := NewState(pool, &config.Config{PublicURL: "https://example.in"})
	srv := httptest.NewServer(MCPHandler(st, engine))
	t.Cleanup(srv.Close)

	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "1"}, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	session, err := client.Connect(ctx, &mcp.StreamableClientTransport{
		Endpoint:   srv.URL,
		HTTPClient: &http.Client{Transport: bearerTransport{token: token}},
	}, nil)
	if err != nil {
		cancel()
		t.Fatalf("connect: %v", err)
	}
	return session, func() {
		_ = session.Close()
		cancel()
	}
}

func toolNames(t *testing.T, session *mcp.ClientSession) []string {
	t.Helper()
	res, err := session.ListTools(context.Background(), nil)
	if err != nil {
		t.Fatalf("list tools: %v", err)
	}
	names := []string{}
	for _, tool := range res.Tools {
		names = append(names, tool.Name)
	}
	return names
}

func TestAReadKeySeesTheBookAndNotTheWriteTools(t *testing.T) {
	session, done := mcpSession(t, apikeys.ScopeRead)
	defer done()

	names := toolNames(t, session)
	for _, want := range []string{"get_dashboard", "list_positions", "list_decisions", "get_instrument"} {
		if !containsName(names, want) {
			t.Fatalf("read catalogue is missing %s: %v", want, names)
		}
	}
	for _, forbidden := range []string{"approve_decision", "record_trade", "update_settings"} {
		if containsName(names, forbidden) {
			t.Fatalf("a read key was offered %s: %v", forbidden, names)
		}
	}

	res, err := session.CallTool(context.Background(), &mcp.CallToolParams{Name: "get_dashboard"})
	if err != nil {
		t.Fatalf("get_dashboard: %v", err)
	}
	text := resultText(t, res)
	if !strings.Contains(text, "INFY.NS") || !strings.Contains(text, "positions") {
		t.Fatalf("the dashboard did not carry the book: %s", text)
	}
}

func TestAWriteKeyCanRecordAFillOverMCP(t *testing.T) {
	session, done := mcpSession(t, apikeys.ScopeWrite)
	defer done()

	names := toolNames(t, session)
	for _, want := range []string{"record_trade", "approve_decision", "update_settings"} {
		if !containsName(names, want) {
			t.Fatalf("write catalogue is missing %s: %v", want, names)
		}
	}

	res, err := session.CallTool(context.Background(), &mcp.CallToolParams{
		Name:      "record_trade",
		Arguments: map[string]any{"symbol": "INFY.NS", "side": "buy", "qty": 3, "price": 120.0},
	})
	if err != nil {
		t.Fatalf("record_trade: %v", err)
	}
	if res.IsError {
		t.Fatalf("record_trade refused: %s", resultText(t, res))
	}

	res, err = session.CallTool(context.Background(), &mcp.CallToolParams{Name: "list_positions"})
	if err != nil {
		t.Fatalf("list_positions: %v", err)
	}
	if !strings.Contains(resultText(t, res), `"qty":13`) {
		t.Fatalf("the fill did not move the position: %s", resultText(t, res))
	}
}

func TestAReadKeyCannotCallAWriteToolEvenIfItAsks(t *testing.T) {
	session, done := mcpSession(t, apikeys.ScopeRead)
	defer done()

	res, err := session.CallTool(context.Background(), &mcp.CallToolParams{
		Name:      "record_trade",
		Arguments: map[string]any{"symbol": "INFY.NS", "side": "buy", "qty": 1, "price": 100.0},
	})
	if err == nil && !res.IsError {
		t.Fatal("a read key recorded a fill")
	}
}

func TestARevokedKeyCannotConnect(t *testing.T) {
	pool, engine, _ := setup(t)
	token, key, err := apikeys.New(pool, "gone", apikeys.ScopeRead, nil, time.Now().UTC())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := apikeys.Revoke(pool, key.ID, time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	st := NewState(pool, &config.Config{PublicURL: "https://example.in"})
	srv := httptest.NewServer(MCPHandler(st, engine))
	defer srv.Close()

	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "1"}, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	transport := &mcp.StreamableClientTransport{
		Endpoint:   srv.URL,
		HTTPClient: &http.Client{Transport: bearerTransport{token: token}},
	}
	if _, err := client.Connect(ctx, transport, nil); err == nil {
		t.Fatal("a revoked key connected")
	}
}

func resultText(t *testing.T, res *mcp.CallToolResult) string {
	t.Helper()
	if res == nil {
		return ""
	}
	out := strings.Builder{}
	for _, content := range res.Content {
		if text, ok := content.(*mcp.TextContent); ok {
			out.WriteString(text.Text)
		}
	}
	return out.String()
}

func containsName(names []string, want string) bool {
	for _, n := range names {
		if n == want {
			return true
		}
	}
	return false
}
