package llm

import (
	"database/sql"
	"time"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

// One row per outbound model call, success or failure.
type ApiCallRow struct {
	ConversationID   *string
	Round            *int64
	Endpoint         string
	Trigger          *string
	Purpose          *string
	PromptTemplate   *string
	PromptHash       *string
	RequestJSON      *string
	ResponseJSON     *string
	HTTPStatus       *int64
	PromptTokens     *int64
	CompletionTokens *int64
	LatencyMS        *int64
	ValidatorVerdict *string
	Error            *string
}

func RecordCall(pool *sql.DB, row ApiCallRow) error {
	_, err := pool.Exec(
		`INSERT INTO api_calls (id, ts, conversation_id, round, endpoint, purpose, prompt_template,
            prompt_hash, request_json, response_json, http_status, prompt_tokens,
            completion_tokens, latency_ms, validator_verdict, error, trigger)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		ids.New(), tstamp.Now(), row.ConversationID, row.Round, row.Endpoint, row.Purpose,
		row.PromptTemplate, row.PromptHash, row.RequestJSON, row.ResponseJSON, row.HTTPStatus,
		row.PromptTokens, row.CompletionTokens, row.LatencyMS, row.ValidatorVerdict, row.Error,
		row.Trigger,
	)
	return err
}

// Delete calls older than days. The audit keeps three months; older rows are the bulk of
// the file and nothing reads them.
func PruneCalls(pool *sql.DB, days int64) (int64, error) {
	cutoff := time.Now().UTC().AddDate(0, 0, -int(days)).Format("2006-01-02T15:04:05.000000000+00:00")
	res, err := pool.Exec("DELETE FROM api_calls WHERE ts < ?", cutoff)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}
