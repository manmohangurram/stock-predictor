package engine

import (
	"bytes"
	"log/slog"
	"strings"
	"testing"
	"time"

	"stock-predictor/internal/config"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
)

type passingModel struct {
	calls int
}

func (m *passingModel) Post(url, body string) (llm.HttpReply, error) {
	m.calls++
	return llm.HttpReply{
		Status: 200,
		Body: `{"choices":[{"message":{"content":"{\"pass_reason\":\"nothing worth doing today\"}"}}],` +
			`"usage":{"prompt_tokens":10,"completion_tokens":5}}`,
	}, nil
}

// A manual run is closed by the caller that opened it. When the plan call also closed it, every
// manual run printed "run finished" twice with the same id.
func TestAManualPlanClosesItsRunOnce(t *testing.T) {
	p := pool(t)
	if err := rules.Seed(p); err != nil {
		t.Fatal(err)
	}
	t.Setenv(config.KeyEnvName("deepseek"), "test-key")

	var logbuf bytes.Buffer
	prev := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&logbuf, &slog.HandlerOptions{Level: slog.LevelInfo})))
	t.Cleanup(func() { slog.SetDefault(prev) })

	model := &passingModel{}
	deps := &EngineDeps{Pool: p, Runtime: rules.DefaultRuntime(), Transport: model}

	runID, err := runs.Start(p, runs.PlanStock, runs.TriggerManual, nil)
	if err != nil {
		t.Fatal(err)
	}
	planned, note, err := planBookWith(deps, time.Now().UTC(), llm.ScopeStock, &runID, "", false)
	if err != nil {
		t.Fatal(err)
	}
	if planned {
		t.Fatal("the stub passed, so nothing should have been stored")
	}
	if model.calls != 1 {
		t.Fatalf("the plan asked the model %d times, wanted 1", model.calls)
	}

	if err := runs.Finish(p, runID, note, nil); err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(logbuf.String(), `msg="run finished"`); got != 1 {
		t.Fatalf("one run must close once, got %d lines:\n%s", got, logbuf.String())
	}
}
