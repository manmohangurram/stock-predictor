package engine

import (
	"errors"
	"testing"
	"time"

	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/store"
)

type refusingMarket struct{}

func (refusingMarket) Bars(string) ([]models.Bar, error) {
	return nil, errors.New("chart: http status 429")
}

func (refusingMarket) Intraday(string, string, string) ([]models.Bar, error) {
	return nil, errors.New("chart: http status 429")
}

func (refusingMarket) News(string) ([]models.Headline, error)      { return nil, nil }
func (refusingMarket) Constituents() ([]market.Constituent, error) { return nil, nil }

func TestARefusedDailySeriesFallsBackToTheStoredOne(t *testing.T) {
	p := pool(t)
	instrument := models.Instrument{
		ID: "01a1f000-0000-7000-8000-000000000051", Symbol: "X.NS", Name: "X", Class: models.ClassStock,
	}
	mustExec(t, p, `INSERT INTO instruments (id, symbol, class, name, currency, active)
        VALUES (?, 'X.NS', 'stock', 'X', 'INR', 1)`, instrument.ID)
	if err := store.Bars(p, instrument.ID, []models.Bar{
		{Ts: "2026-11-10T00:00:00.000000000+00:00", Open: 100, High: 101, Low: 99, Close: 100},
	}); err != nil {
		t.Fatal(err)
	}

	deps := &EngineDeps{Pool: p, Market: refusingMarket{}, Runtime: rules.DefaultRuntime()}
	g, err := gather(deps, instrument, time.Date(2026, 11, 13, 5, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	if !g.staleDaily {
		t.Fatal("the stored series was not marked stale")
	}
	if g.indOK {
		t.Fatal("one bar is not enough for indicators")
	}
	if len(g.bars) != 1 || g.bars[0].Close != 100 {
		t.Fatalf("%+v", g.bars)
	}
}
