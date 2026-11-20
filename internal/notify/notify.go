// Package notify is the push channel: ntfy, priorities, and a dedupe key that survives a
// restart.
package notify

import (
	"crypto/sha256"
	"database/sql"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"strings"
	"sync"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/tstamp"
)

type Priority string

const (
	PriorityHigh    Priority = "high"
	PriorityDefault Priority = "default"
)

type Notification struct {
	Title     string   `json:"title"`
	Body      string   `json:"body"`
	Priority  Priority `json:"priority"`
	DedupeKey string   `json:"dedupe_key"`
	SignalID  *string  `json:"signal_id"`
	// Used for the tap-through link; empty when there is nothing to open.
	Symbol string `json:"symbol"`
}

// Copy follows the design system: one line, figures in mono, no adjectives.
// BLOCKED never produces a Notification — there is nothing to do about it.
// Name is what a person recognises ("Infosys"), symbol is for the link and the footnote — a
// push that says only INFY.NS makes you stop and translate before you know what happened.
func FromCandidate(symbol, name, currency string, c models.Candidate) *Notification {
	if c.Action != models.ActionBuy && c.Action != models.ActionSell {
		return nil
	}
	var title string
	priority := PriorityDefault
	if c.Action == models.ActionSell {
		title = fmt.Sprintf("SELL · %s · %.0f units", name, c.Qty)
		priority = PriorityHigh
	} else {
		title = fmt.Sprintf("BUY · %s · %s units at %.2f", name, fmtQty(c.Qty), c.Entry)
	}

	var body string
	if c.Action == models.ActionSell {
		// Say which exit fired: a stop-out and a target reached are opposite feelings.
		why := ""
		if c.Stop != nil && c.Entry <= *c.Stop {
			why = fmt.Sprintf("stop %.2f hit, ", *c.Stop)
		} else if c.Target != nil && c.Entry >= *c.Target {
			why = fmt.Sprintf("target %.2f reached — take it, ", *c.Target)
		}
		body = fmt.Sprintf("%snow %.2f · %s · %s", why, c.Entry, c.TimingNote, symbol)
	} else {
		// A buy is only actionable if it says where to get out on both sides.
		exits := ""
		if c.Stop != nil && c.Target != nil {
			exits = fmt.Sprintf("sell if ≤ %.2f · take profit ≥ %.2f · ", *c.Stop, *c.Target)
		}
		body = fmt.Sprintf("cost %.2f %s · %s%s · %s", c.Qty*c.Entry, currency, exits, c.TimingNote, symbol)
	}

	return &Notification{
		Title:     title,
		Body:      body,
		Priority:  priority,
		DedupeKey: DedupeKey(symbol, c),
		Symbol:    symbol,
	}
}

func fmtQty(q float64) string {
	if diff := q - round(q); diff < 1e-9 && diff > -1e-9 {
		return fmt.Sprintf("%.0f", q)
	}
	return fmt.Sprintf("%.3f", q)
}

func round(v float64) float64 {
	if v < 0 {
		return float64(int64(v - 0.5))
	}
	return float64(int64(v + 0.5))
}

func DedupeKey(symbol string, c models.Candidate) string {
	h := sha256.New()
	io.WriteString(h, symbol)
	io.WriteString(h, string(c.Action))
	fmt.Fprintf(h, "%.2f", c.Entry)
	if c.Stop != nil {
		fmt.Fprintf(h, "%.2f", *c.Stop)
	}
	return fmt.Sprintf("%x", h.Sum(nil))
}

type Notifier interface {
	Send(n Notification) error

	// False when there is nowhere to push: ntfy with no topic. A decision is still recorded,
	// it just does not reach a phone.
	Configured() bool
}

type Ntfy struct {
	client    *http.Client
	url       string
	token     string
	publicURL string
	// Empty topic means "off", not "post to the root and fail".
	enabled bool
}

func NewNtfy(baseURL, topic, token, publicURL string, rt *rules.Runtime) (*Ntfy, error) {
	client, err := httpx.Client(rt, 15)
	if err != nil {
		return nil, err
	}
	return &Ntfy{
		client:    client,
		url:       strings.TrimRight(baseURL, "/") + "/" + strings.TrimLeft(topic, "/"),
		token:     token,
		publicURL: strings.TrimRight(publicURL, "/"),
		enabled:   strings.TrimSpace(topic) != "",
	}, nil
}

// Build from the settings table, which is what the UI edits.
func NtfyFromSettings(pool *sql.DB, publicURL string) (*Ntfy, error) {
	s, err := rules.GetSettings(pool)
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(s.NtfyTopic) == "" {
		return nil, nil
	}
	token := ""
	if t, err := rules.NtfyToken(pool); err == nil && t != nil {
		token = *t
	}
	rt, err := rules.LoadRuntime(pool)
	if err != nil {
		return nil, err
	}
	return NewNtfy(s.NtfyURL, s.NtfyTopic, token, publicURL, &rt)
}

// ntfy headers are latin-1; the copy uses non-ASCII separators, so fold them down rather than
// letting the request be rejected.
func encodeHeader(s string) string {
	var b strings.Builder
	for _, r := range s {
		if r < 128 {
			b.WriteRune(r)
		} else {
			b.WriteByte('-')
		}
	}
	return b.String()
}

// The headers and body ntfy expects, built separately so they can be asserted without a
// network round trip. Kept in one place: a header typo used to mean a silently missing alert.
func Payload(n Notification, publicURL string) (map[string]string, string) {
	tags := "chart_with_upwards_trend"
	if n.Priority == PriorityHigh {
		tags = "rotating_light"
	}
	headers := map[string]string{
		"Title":    encodeHeader(n.Title),
		"Priority": string(n.Priority),
		"Tags":     tags,
	}
	if publicURL != "" && n.Symbol != "" {
		headers["Click"] = publicURL + "/i/" + n.Symbol
	}
	return headers, n.Body
}

func (n *Ntfy) Configured() bool { return n.enabled }

func (n *Ntfy) Send(notification Notification) error {
	headers, body := Payload(notification, n.publicURL)
	req, err := http.NewRequest(http.MethodPost, n.url, strings.NewReader(body))
	if err != nil {
		return err
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	// A protected topic rejects anonymous posts; open topics ignore the header.
	if n.token != "" {
		req.Header.Set("Authorization", "Bearer "+n.token)
	}
	resp, err := n.client.Do(req)
	if err != nil {
		return fmt.Errorf("ntfy: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		detail, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("ntfy: http %d %s", resp.StatusCode, strings.TrimSpace(string(detail)))
	}
	return nil
}

func (n *Ntfy) SendTest(notification Notification) error { return n.Send(notification) }

type FakeNotifier struct {
	mu   sync.Mutex
	Sent []Notification
	Fail bool
}

func FailingNotifier() *FakeNotifier { return &FakeNotifier{Fail: true} }

func (f *FakeNotifier) Count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.Sent)
}

func (f *FakeNotifier) Configured() bool { return true }

func (f *FakeNotifier) Send(n Notification) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.Fail {
		return fmt.Errorf("ntfy: simulated outage")
	}
	f.Sent = append(f.Sent, n)
	return nil
}

// The row is written only after a successful send, so a failed push is retried on the next
// tick instead of being silently swallowed.
func Dispatch(pool *sql.DB, notifier Notifier, n Notification) (bool, error) {
	if !notifier.Configured() {
		slog.Debug("push skipped: no channel configured", "title", n.Title)
		return false, nil
	}
	var existing string
	err := pool.QueryRow("SELECT id FROM notifications WHERE dedupe_key = ?", n.DedupeKey).Scan(&existing)
	if err == nil {
		slog.Debug("push skipped: already sent", "dedupe_key", n.DedupeKey)
		return false, nil
	}
	if err != sql.ErrNoRows {
		return false, err
	}

	if err := notifier.Send(n); err != nil {
		slog.Warn("push failed", "title", n.Title, "error", err)
		return false, err
	}
	slog.Info("push sent", "title", n.Title, "priority", n.Priority, "symbol", n.Symbol)

	if _, err := pool.Exec(
		"INSERT INTO notifications (id, signal_id, sent_at, channel, dedupe_key) VALUES (?, ?, ?, 'ntfy', ?)",
		ids.New(), n.SignalID, tstamp.Now(), n.DedupeKey,
	); err != nil {
		return false, err
	}
	return true, nil
}
