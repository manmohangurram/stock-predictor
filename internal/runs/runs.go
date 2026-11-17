// Package runs records what the scheduler (or a person) started and how it ended.
//
// A run is opened before the work and closed after it, so the audit page can answer "is it
// running?" — something api_calls cannot, because a call row only exists once the reply lands.
package runs

import (
	"database/sql"
	"fmt"
	"log/slog"
	"time"

	"stock-predictor/internal/tstamp"
)

// What was run. The string is stored, so a new kind needs no migration.
type Prompt string

const (
	Poll      Prompt = "poll"
	PlanStock Prompt = "plan_stock"
	PlanFund  Prompt = "plan_fund"
	// The trade purse's plan: every two hours in session, replacing the old per-stock pass.
	PlanTrade Prompt = "plan_trade"
	// Historical: the per-stock short-term pass, replaced by PlanTrade. Old rows still read.
	Short  Prompt = "short"
	Decide Prompt = "decide"
	// A buy that did not fit the cash, asking what to sell.
	Rotate Prompt = "rotate"
)

func ParsePrompt(raw string) (Prompt, bool) {
	switch p := Prompt(raw); p {
	case Poll, PlanStock, PlanFund, PlanTrade, Short, Decide, Rotate:
		return p, true
	}
	return "", false
}

type Trigger string

const (
	TriggerSchedule Trigger = "schedule"
	TriggerChange   Trigger = "change"
	TriggerManual   Trigger = "manual"
)

type Run struct {
	ID             int64   `json:"id"`
	Prompt         string  `json:"prompt"`
	Trigger        string  `json:"trigger"`
	Status         string  `json:"status"`
	StartedAt      string  `json:"started_at"`
	FinishedAt     *string `json:"finished_at"`
	Summary        *string `json:"summary"`
	ConversationID *string `json:"conversation_id"`
	Error          *string `json:"error"`
}

// Open a run. It exists while the work is still going, which is the whole point.
func Start(pool *sql.DB, prompt Prompt, trigger Trigger, symbol *string) (int64, error) {
	var summary *string
	if symbol != nil {
		s := *symbol + " only"
		summary = &s
	}
	var id int64
	err := pool.QueryRow(
		`INSERT INTO runs (prompt, trigger, status, started_at, summary)
         VALUES (?, ?, 'running', ?, ?) RETURNING id`,
		string(prompt), string(trigger), tstamp.Now(), summary,
	).Scan(&id)
	if err == nil {
		slog.Info("run started", "id", id, "prompt", prompt, "trigger", trigger)
	}
	return id, err
}

// Close a run with one line a person can read.
func Finish(pool *sql.DB, id int64, summary string, conversationID *string) error {
	_, err := pool.Exec(
		`UPDATE runs SET status = 'done', finished_at = ?, summary = ?,
            conversation_id = COALESCE(?, conversation_id)
         WHERE id = ?`,
		tstamp.Now(), summary, conversationID, id,
	)
	if err == nil {
		slog.Info("run finished", "id", id, "summary", summary)
	}
	return err
}

// Close a run that broke. The error is kept: a run that failed is still a thing that happened.
func Fail(pool *sql.DB, id int64, failure string) error {
	_, err := pool.Exec(
		"UPDATE runs SET status = 'failed', finished_at = ?, error = ? WHERE id = ?",
		tstamp.Now(), failure, id,
	)
	if err == nil {
		slog.Warn("run failed", "id", id, "error", failure)
	}
	return err
}

// A run still marked running when the process died is not running any more. Called at boot so
// the page never shows a spinner that will not stop.
func CloseOrphans(pool *sql.DB, now time.Time) (int64, error) {
	res, err := pool.Exec(
		`UPDATE runs SET status = 'failed', finished_at = ?,
            error = COALESCE(error, 'the process stopped before this finished')
         WHERE status = 'running'`,
		tstamp.From(now),
	)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}

func Recent(pool *sql.DB, limit int64) ([]Run, error) {
	rows, err := pool.Query(
		`SELECT id, prompt, trigger, status, started_at, finished_at, summary,
                conversation_id, error
         FROM runs ORDER BY id DESC LIMIT ?`, limit,
	)
	if err != nil {
		return nil, err
	}
	return scanRuns(rows)
}

// The most recent run of each prompt — one row per scheduled kind, however long ago it ran.
func LatestByPrompt(pool *sql.DB) ([]Run, error) {
	rows, err := pool.Query(
		`SELECT id, prompt, trigger, status, started_at, finished_at, summary,
                conversation_id, error
         FROM runs r
         WHERE r.id = (SELECT MAX(id) FROM runs x WHERE x.prompt = r.prompt)
         ORDER BY r.prompt`,
	)
	if err != nil {
		return nil, err
	}
	return scanRuns(rows)
}

func Running(pool *sql.DB) (int64, error) {
	var n int64
	err := pool.QueryRow("SELECT COUNT(*) FROM runs WHERE status = 'running'").Scan(&n)
	return n, err
}

func scanRuns(rows *sql.Rows) ([]Run, error) {
	defer rows.Close()
	out := []Run{}
	for rows.Next() {
		var r Run
		var finished, summary, conversation, errText sql.NullString
		if err := rows.Scan(&r.ID, &r.Prompt, &r.Trigger, &r.Status, &r.StartedAt,
			&finished, &summary, &conversation, &errText); err != nil {
			return nil, err
		}
		if finished.Valid {
			r.FinishedAt = &finished.String
		}
		if summary.Valid {
			r.Summary = &summary.String
		}
		if conversation.Valid {
			r.ConversationID = &conversation.String
		}
		if errText.Valid {
			r.Error = &errText.String
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

// A request to run something now, sent from the API to the loop that owns the clients.
type Manual struct {
	Kind   ManualKind
	Symbol string
}

type ManualKind string

const (
	ManualPoll      ManualKind = "poll"
	ManualPlanStock ManualKind = "plan_stock"
	ManualPlanFund  ManualKind = "plan_fund"
	ManualPlanTrade ManualKind = "plan_trade"
	// One instrument, asked on its own — the precise version of a poll.
	ManualDecide ManualKind = "decide"
)

func (m Manual) String() string {
	if m.Kind == ManualDecide {
		return fmt.Sprintf("decide %s", m.Symbol)
	}
	return string(m.Kind)
}
