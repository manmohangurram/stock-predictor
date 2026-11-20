package api

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"stock-predictor/internal/apikeys"
	"stock-predictor/internal/auth"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

// The MCP endpoint: the same server, the same handlers, behind a key instead of a click. Every tool
// dispatches through the REST router in memory, so a write is the transaction the page would have
// run - not a second path that can drift from it.

type mcpKeyContext struct{}

// mcpTokenContext carries the access token the tools use to call the app's own router, which is
// gated like every other REST route.
type mcpTokenContext struct{}

func accessFromRequest(r *http.Request) string {
	token, _ := r.Context().Value(mcpTokenContext{}).(string)
	return token
}

func keyFromRequest(r *http.Request) *apikeys.Key {
	key, _ := r.Context().Value(mcpKeyContext{}).(*apikeys.Key)
	return key
}

func bearerToken(r *http.Request) string {
	const prefix = "Bearer "
	raw := r.Header.Get("Authorization")
	if len(raw) <= len(prefix) || !strings.EqualFold(raw[:len(prefix)], prefix) {
		return ""
	}
	return strings.TrimSpace(raw[len(prefix):])
}

// MCPHandler authenticates the request and hands it to the SDK's stateless transport. The server is
// built per request from the key's scope, so a read key's catalogue does not contain the write
// tools at all.
func MCPHandler(st *AppState, engine http.Handler) http.Handler {
	handler := mcp.NewStreamableHTTPHandler(
		func(r *http.Request) *mcp.Server {
			return newMCPServer(st, engine, keyFromRequest(r), accessFromRequest(r))
		},
		&mcp.StreamableHTTPOptions{Stateless: true, JSONResponse: true},
	)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		key, err := verifyKey(st.Pool, bearerToken(r))
		if err != nil {
			slog.Warn("mcp request refused", "reason", err, "remote", r.RemoteAddr)
			w.Header().Set("WWW-Authenticate", `Bearer realm="stock-predictor"`)
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		ctx := context.WithValue(r.Context(), mcpKeyContext{}, key)
		if admin, aerr := auth.Current(st.Pool); aerr == nil {
			if access, terr := auth.IssueAccess(signingKey(st), admin.ID, time.Now().UTC()); terr == nil {
				ctx = context.WithValue(ctx, mcpTokenContext{}, access)
			}
		}
		handler.ServeHTTP(w, r.WithContext(ctx))
	})
}

type restCaller struct {
	engine http.Handler
	token  string
}

// call dispatches one request through the app's own router and decodes the JSON answer.
func (c restCaller) call(method, path string, body any) (json.RawMessage, error) {
	var reader io.Reader
	if body != nil {
		encoded, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		reader = bytes.NewReader(encoded)
	}
	req := httptest.NewRequest(method, path, reader)
	if c.token != "" {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	rec := httptest.NewRecorder()
	c.engine.ServeHTTP(rec, req)
	out := rec.Body.Bytes()
	if rec.Code >= 400 {
		var apiErr struct {
			Error string `json:"error"`
		}
		_ = json.Unmarshal(out, &apiErr)
		if apiErr.Error == "" {
			apiErr.Error = strings.TrimSpace(string(out))
		}
		return nil, fmt.Errorf("%s", apiErr.Error)
	}
	return json.RawMessage(out), nil
}

func (c restCaller) get(path string) (json.RawMessage, error) {
	return c.call(http.MethodGet, path, nil)
}

// audited wraps a tool so every call lands in mcp_calls, whatever the outcome.
func audited(st *AppState, key *apikeys.Key, tool string, args any, fn func() (json.RawMessage, error)) (*mcp.CallToolResult, any, error) {
	started := time.Now()
	out, err := fn()
	status, errText := "ok", ""
	if err != nil {
		status, errText = "error", err.Error()
	}
	raw, _ := json.Marshal(redact(args))
	if _, aerr := st.Pool.Exec(
		`INSERT INTO mcp_calls (id, ts, key_id, tool, args_json, status, error, ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		ids.New(), tstamp.Now(), key.ID, tool, string(raw), status, nullIfEmptyString(errText),
		time.Since(started).Milliseconds(),
	); aerr != nil {
		slog.Warn("mcp audit row failed", "tool", tool, "error", aerr)
	}
	if err != nil {
		return &mcp.CallToolResult{
			IsError: true,
			Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}},
		}, nil, nil
	}
	return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: string(out)}}}, nil, nil
}

// redact keeps secrets out of the audit table: the settings patch may carry them, and nothing else
// should ever store them.
func redact(args any) any {
	encoded, err := json.Marshal(args)
	if err != nil {
		return map[string]any{}
	}
	var fields map[string]any
	if json.Unmarshal(encoded, &fields) != nil {
		return map[string]any{}
	}
	for _, secret := range []string{"llm_key", "ntfy_token", "proxy_pass", "api_key", "token"} {
		if _, ok := fields[secret]; ok {
			fields[secret] = "redacted"
		}
	}
	return fields
}

func nullIfEmptyString(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}

func percentEscape(symbol string) string { return url.PathEscape(symbol) }

// tool stamps the shape every tool shares: a title a client can show, a summary that says what the
// call costs when it costs anything, and one concrete example the model can copy.
func tool(name, title, summary, example string, readOnly, destructive bool) *mcp.Tool {
	t := &mcp.Tool{
		Name:        name,
		Title:       title,
		Description: summary + "\n\nExample input: " + example,
		Annotations: &mcp.ToolAnnotations{ReadOnlyHint: readOnly},
	}
	if destructive {
		yes := true
		t.Annotations.DestructiveHint = &yes
	}
	return t
}

// newMCPServer builds the catalogue for one key. Write tools exist only when the key can write.
func newMCPServer(st *AppState, engine http.Handler, key *apikeys.Key, access string) *mcp.Server {
	server := mcp.NewServer(&mcp.Implementation{Name: "stock-predictor", Version: "1"}, nil)
	rest := restCaller{engine: engine, token: access}

	type modeArg struct {
		Mode string `json:"mode,omitempty" jsonschema:"long or trade; default long"`
	}
	pickMode := func(m string) string {
		if m == "trade" {
			return "trade"
		}
		return "long"
	}

	mcp.AddTool(server, tool("get_dashboard", "Read the whole book",
		"The book as the ledger page shows it: the head figures, every open position, the decisions and the recent fills. Start here when the question is about the portfolio as a whole.",
		`{"mode":"long"}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args modeArg) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "get_dashboard", args, func() (json.RawMessage, error) {
				return rest.get("/api/dashboard?mode=" + pickMode(args.Mode) + "&decisions=all")
			})
		})

	mcp.AddTool(server, tool("list_positions", "List holdings",
		"The positions for one purse, with qty, average cost, last price, value and the exit plan.",
		`{"mode":"trade"}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args modeArg) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "list_positions", args, func() (json.RawMessage, error) {
				raw, err := rest.get("/api/dashboard?mode=" + pickMode(args.Mode))
				if err != nil {
					return nil, err
				}
				var payload struct {
					Positions json.RawMessage `json:"positions"`
				}
				if err := json.Unmarshal(raw, &payload); err != nil {
					return nil, err
				}
				return payload.Positions, nil
			})
		})

	mcp.AddTool(server, tool("list_decisions", "List decisions",
		"Proposals the app made for one purse, filtered by state. A decision carries the uuid that approve_decision and reject_decision take.",
		`{"mode":"long","status":"pending"}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Mode   string `json:"mode,omitempty" jsonschema:"long or trade; default long"`
			Status string `json:"status,omitempty" jsonschema:"pending, approved, rejected, all; default pending"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "list_decisions", args, func() (json.RawMessage, error) {
				status := args.Status
				if status == "" {
					status = "pending"
				}
				raw, err := rest.get("/api/dashboard?mode=" + pickMode(args.Mode) + "&decisions=" + url.QueryEscape(status))
				if err != nil {
					return nil, err
				}
				var payload struct {
					Signals json.RawMessage `json:"signals"`
				}
				if err := json.Unmarshal(raw, &payload); err != nil {
					return nil, err
				}
				return payload.Signals, nil
			})
		})

	mcp.AddTool(server, tool("list_trades", "List fills",
		"Fills that were actually recorded, newest first, for the whole book or one symbol. The id here is what edit_trade takes.",
		`{"symbol":"INFY.NS"}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string `json:"symbol,omitempty" jsonschema:"e.g. INFY.NS; blank for the whole book"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "list_trades", args, func() (json.RawMessage, error) {
				if strings.TrimSpace(args.Symbol) == "" {
					return rest.get("/api/trades")
				}
				return rest.get("/api/instruments/" + percentEscape(args.Symbol) + "/trades")
			})
		})

	mcp.AddTool(server, tool("get_instrument", "Read one instrument",
		"One symbol in full: price series, indicators, the exit plan, headlines and its recent decisions.",
		`{"symbol":"RELIANCE.NS"}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string `json:"symbol" jsonschema:"e.g. INFY.NS"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "get_instrument", args, func() (json.RawMessage, error) {
				return rest.get("/api/instruments/" + percentEscape(args.Symbol) + "/analysis")
			})
		})

	mcp.AddTool(server, tool("list_archived", "List archived instruments",
		"Instruments a person archived, with their totals. Nothing is restored automatically.",
		`{}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct{}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "list_archived", args, func() (json.RawMessage, error) {
				return rest.get("/api/archived")
			})
		})

	mcp.AddTool(server, tool("get_market", "Market state and schedule",
		"Whether the exchange is trading, when it next opens, the prompt schedule and anything running now.",
		`{}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct{}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "get_market", args, func() (json.RawMessage, error) {
				market, err := rest.get("/api/market")
				if err != nil {
					return nil, err
				}
				schedule, err := rest.get("/api/schedule")
				if err != nil {
					return nil, err
				}
				combined, _ := json.Marshal(map[string]json.RawMessage{"market": market, "schedule": schedule})
				return combined, nil
			})
		})

	mcp.AddTool(server, tool("list_runs", "List runs",
		"Recent scheduler runs and how each ended, newest first.",
		`{"limit":10}`, true, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Limit int `json:"limit,omitempty" jsonschema:"default 25, at most 200"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "list_runs", args, func() (json.RawMessage, error) {
				limit := args.Limit
				if limit < 1 || limit > 200 {
					limit = 25
				}
				return rest.get(fmt.Sprintf("/api/runs?limit=%d", limit))
			})
		})

	if key == nil || !key.CanWrite() {
		return server
	}
	registerWriteTools(server, st, key, rest)
	return server
}

func registerWriteTools(server *mcp.Server, st *AppState, key *apikeys.Key, rest restCaller) {
	mcp.AddTool(server, tool("approve_decision", "Approve a decision",
		"Approve a pending decision and post the fill it implies. Money moves: the position, the fill row and the cash ledger change together in one transaction. Pass qty and price only when the fill differs from the proposal; anything already decided is refused.",
		`{"id":"<decision uuid from list_decisions>","qty":11,"price":1410}`, false, true),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			ID    string   `json:"id" jsonschema:"the decision uuid from list_decisions"`
			Qty   *float64 `json:"qty,omitempty" jsonschema:"units actually filled; defaults to the proposal"`
			Price *float64 `json:"price,omitempty" jsonschema:"price actually paid; defaults to the entry"`
			At    *string  `json:"at,omitempty" jsonschema:"when it filled, RFC3339; default now"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "approve_decision", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/signals/"+percentEscape(args.ID)+"/approve", map[string]any{
					"qty": args.Qty, "price": args.Price, "at": args.At,
				})
			})
		})

	mcp.AddTool(server, tool("reject_decision", "Reject a decision",
		"Reject a pending decision with a reason. No money moves and no fill is written.",
		`{"id":"<decision uuid>","reason":"earnings tomorrow"}`, false, true),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			ID     string  `json:"id" jsonschema:"the decision uuid"`
			Reason *string `json:"reason,omitempty"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "reject_decision", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/signals/"+percentEscape(args.ID)+"/reject",
					map[string]any{"reason": args.Reason})
			})
		})

	mcp.AddTool(server, tool("record_trade", "Record a fill",
		"Record a fill that actually happened: position, fill row and cash ledger in one transaction. Use list_positions first to know what is held, and side sell cannot exceed it.",
		`{"symbol":"INFY.NS","side":"buy","qty":5,"price":1425.5,"mode":"long"}`, false, true),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string  `json:"symbol" jsonschema:"e.g. INFY.NS"`
			Side   string  `json:"side" jsonschema:"buy or sell"`
			Qty    float64 `json:"qty" jsonschema:"units"`
			Price  float64 `json:"price" jsonschema:"price per unit"`
			At     *string `json:"at,omitempty" jsonschema:"when, RFC3339 or YYYY-MM-DD; default now"`
			Note   *string `json:"note,omitempty"`
			Mode   string  `json:"mode,omitempty" jsonschema:"long or trade; default long"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "record_trade", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/instruments/"+percentEscape(args.Symbol)+"/trades", map[string]any{
					"side": args.Side, "qty": args.Qty, "price": args.Price,
					"at": args.At, "note": args.Note, "mode": pickModeOr(args.Mode),
				})
			})
		})

	mcp.AddTool(server, tool("edit_trade", "Correct a fill",
		"Correct a recorded fill. Only the fields you pass change; the position and the cash move by the difference, in one transaction.",
		`{"id":"<fill uuid from list_trades>","price":1420}`, false, true),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			ID    string   `json:"id" jsonschema:"the fill uuid from list_trades"`
			Qty   *float64 `json:"qty,omitempty"`
			Price *float64 `json:"price,omitempty"`
			At    *string  `json:"at,omitempty"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "edit_trade", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPatch, "/api/trades/"+percentEscape(args.ID), map[string]any{
					"qty": args.Qty, "price": args.Price, "at": args.At,
				})
			})
		})

	mcp.AddTool(server, tool("add_instrument", "Add an instrument",
		"Add an NSE or BSE symbol, or a fund by AMFI scheme code, and fetch its history. A non-Indian listing is refused: it would be priced in dollars and read as rupees.",
		`{"code":"WIPRO.NS"}`, false, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Code  string  `json:"code" jsonschema:"e.g. WIPRO.NS, or an AMFI scheme code"`
			Name  *string `json:"name,omitempty"`
			Class *string `json:"class,omitempty" jsonschema:"stock or mutual_fund"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "add_instrument", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/instruments", map[string]any{
					"code": args.Code, "name": args.Name, "class": args.Class,
				})
			})
		})

	mcp.AddTool(server, tool("restore_instrument", "Restore an archived instrument",
		"Bring an archived instrument back into the book.",
		`{"symbol":"INFY.NS"}`, false, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string `json:"symbol"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "restore_instrument", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/instruments/"+percentEscape(args.Symbol)+"/restore", nil)
			})
		})

	mcp.AddTool(server, tool("set_target_profit", "Set a profit expectation",
		"Set the profit percentage that closes a holding, inside the pot's band. Pass null to fall back to the band.",
		`{"symbol":"INFY.NS","pct":12.5}`, false, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string   `json:"symbol"`
			Pct    *float64 `json:"pct" jsonschema:"null clears it back to the pot's band"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "set_target_profit", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPut, "/api/instruments/"+percentEscape(args.Symbol)+"/target-profit",
					map[string]any{"target_profit_pct": args.Pct})
			})
		})

	mcp.AddTool(server, tool("update_settings", "Change settings",
		"Change settings the way the Settings page does: budgets, limits, provider and model, the provider key, ntfy, cadence, the proxy. Only the keys you pass change. Secrets are written and never returned. Invalid values are refused with the reason.",
		`{"poll_secs":300,"daily_cap":200}`, false, false),
		func(ctx context.Context, req *mcp.CallToolRequest, args map[string]any) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "update_settings", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPut, "/api/settings", args)
			})
		})
}

func pickModeOr(m string) string {
	if m == "trade" {
		return "trade"
	}
	return "long"
}
