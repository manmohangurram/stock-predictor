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
	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

// The MCP endpoint: the same server, the same handlers, behind a key instead of a click. Every tool
// dispatches through the REST router in memory, so a write is the transaction the page would have
// run - not a second path that can drift from it.

type mcpKeyContext struct{}

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
		func(r *http.Request) *mcp.Server { return newMCPServer(st, engine, keyFromRequest(r)) },
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
		handler.ServeHTTP(w, r.WithContext(ctx))
	})
}

type restCaller struct {
	engine http.Handler
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

// newMCPServer builds the catalogue for one key. Write tools exist only when the key can write.
func newMCPServer(st *AppState, engine http.Handler, key *apikeys.Key) *mcp.Server {
	server := mcp.NewServer(&mcp.Implementation{Name: "stock-predictor", Version: "1"}, nil)
	rest := restCaller{engine: engine}

	type modeArg struct {
		Mode string `json:"mode,omitempty" jsonschema:"long or trade; default long"`
	}
	pickMode := func(m string) string {
		if m == "trade" {
			return "trade"
		}
		return "long"
	}

	mcp.AddTool(server, &mcp.Tool{Name: "get_dashboard", Description: "The book as the ledger page shows it: figures, positions, decisions and recent fills."},
		func(ctx context.Context, req *mcp.CallToolRequest, args modeArg) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "get_dashboard", args, func() (json.RawMessage, error) {
				return rest.get("/api/dashboard?mode=" + pickMode(args.Mode) + "&decisions=all")
			})
		})

	mcp.AddTool(server, &mcp.Tool{Name: "list_positions", Description: "Open and closed holdings for one purse."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "list_decisions", Description: "Proposals (decisions) for one purse, filtered by state."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "list_trades", Description: "Recorded fills, newest first, optionally for one symbol."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "get_instrument", Description: "One symbol: price series, indicators, exit plan, headlines and its last decisions."},
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string `json:"symbol" jsonschema:"e.g. INFY.NS"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "get_instrument", args, func() (json.RawMessage, error) {
				return rest.get("/api/instruments/" + percentEscape(args.Symbol) + "/analysis")
			})
		})

	mcp.AddTool(server, &mcp.Tool{Name: "list_archived", Description: "Instruments a person archived, with their totals."},
		func(ctx context.Context, req *mcp.CallToolRequest, args struct{}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "list_archived", args, func() (json.RawMessage, error) {
				return rest.get("/api/archived")
			})
		})

	mcp.AddTool(server, &mcp.Tool{Name: "get_market", Description: "Market state, next open, the schedule and anything running."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "list_runs", Description: "Recent scheduler runs and how they ended."},
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
	mcp.AddTool(server, &mcp.Tool{Name: "approve_decision", Description: "Approve a pending decision and record the fill it implies."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "reject_decision", Description: "Reject a pending decision, with a reason."},
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			ID     string  `json:"id" jsonschema:"the decision uuid"`
			Reason *string `json:"reason,omitempty"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "reject_decision", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/signals/"+percentEscape(args.ID)+"/reject",
					map[string]any{"reason": args.Reason})
			})
		})

	mcp.AddTool(server, &mcp.Tool{Name: "record_trade", Description: "Record a fill you actually got: one transaction over the position, the fill and the cash."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "edit_trade", Description: "Correct a recorded fill; the position and the cash move by the difference."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "add_instrument", Description: "Add an NSE/BSE symbol (or a fund by AMFI code) and fetch its history."},
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

	mcp.AddTool(server, &mcp.Tool{Name: "restore_instrument", Description: "Bring an archived instrument back into the book."},
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string `json:"symbol"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "restore_instrument", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPost, "/api/instruments/"+percentEscape(args.Symbol)+"/restore", nil)
			})
		})

	mcp.AddTool(server, &mcp.Tool{Name: "set_target_profit", Description: "Set the profit expectation that closes a holding."},
		func(ctx context.Context, req *mcp.CallToolRequest, args struct {
			Symbol string   `json:"symbol"`
			Pct    *float64 `json:"pct" jsonschema:"null clears it back to the pot's band"`
		}) (*mcp.CallToolResult, any, error) {
			return audited(st, key, "set_target_profit", args, func() (json.RawMessage, error) {
				return rest.call(http.MethodPut, "/api/instruments/"+percentEscape(args.Symbol)+"/target-profit",
					map[string]any{"target_profit_pct": args.Pct})
			})
		})

	mcp.AddTool(server, &mcp.Tool{Name: "update_settings", Description: "Change settings the same way the Settings page does: budgets, limits, provider and key, ntfy, cadence. Secrets are written, never returned."},
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
