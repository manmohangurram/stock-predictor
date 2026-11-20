package api

import (
	"database/sql"
	"fmt"
	"log/slog"
	"net/http"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/ledger"
	"stock-predictor/internal/models"
	"stock-predictor/internal/notify"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
	"stock-predictor/internal/trades"
)

func httpxClientFor(rt *rules.Runtime) (*http.Client, error) {
	return httpx.Client(rt, 20)
}

func archiveInstrument(c *gin.Context, st *AppState) {
	symbol := c.Param("symbol")
	var id, active int64
	var archived sql.NullString
	err := st.Pool.QueryRow("SELECT id, active, archived_at FROM instruments WHERE symbol = ?", symbol).
		Scan(&id, &active, &archived)
	if err == sql.ErrNoRows {
		respondErr(c, notFound("no instrument "+symbol))
		return
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if archived.Valid {
		respondErr(c, badRequest(symbol+" is already archived"))
		return
	}
	var held float64
	if err := st.Pool.QueryRow(
		`SELECT CAST(COALESCE(SUM(qty), 0) AS REAL) FROM positions
         WHERE instrument_id = ? AND state <> 'closed'`, id).Scan(&held); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if held > 0 {
		respondErr(c, badRequest(fmt.Sprintf(
			"cannot archive %s while %v units are held — sell it first", symbol, held)))
		return
	}
	tx, err := st.Pool.Begin()
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	// archived_at is the record of a decision; active = 0 alone is also what discovery writes.
	if _, err := tx.Exec("UPDATE instruments SET active = 0, archived_at = ? WHERE id = ?",
		tstampNow(), id); err != nil {
		tx.Rollback()
		respondErr(c, internalErr(err))
		return
	}
	for _, q := range []string{
		"DELETE FROM bars WHERE instrument_id = ?",
		"DELETE FROM news WHERE instrument_id = ?",
		"DELETE FROM llm_memos WHERE instrument_id = ?",
	} {
		if _, err := tx.Exec(q, id); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
	}
	if err := tx.Commit(); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	slog.Info("instrument archived", "symbol", symbol, "id", id)
	c.JSON(http.StatusOK, gin.H{"symbol": symbol, "archived": true, "deleted": false})
}

func restoreInstrument(c *gin.Context, st *AppState) {
	symbol := c.Param("symbol")
	id, err := instrumentIDBySymbol(st.Pool, symbol)
	if err != nil {
		respondErr(c, err)
		return
	}
	res, err := st.Pool.Exec(
		"UPDATE instruments SET active = 1, archived_at = NULL WHERE id = ? AND archived_at IS NOT NULL", id)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if n, _ := res.RowsAffected(); n == 0 {
		respondErr(c, badRequest(symbol+" is not archived — there is nothing to restore"))
		return
	}
	slog.Info("instrument restored", "symbol", symbol, "id", id)
	c.JSON(http.StatusOK, gin.H{"symbol": symbol, "restored": true})
}

func setTargetProfit(c *gin.Context, st *AppState) {
	var body struct {
		TargetProfitPct *float64 `json:"target_profit_pct"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	id, err := instrumentIDBySymbol(st.Pool, c.Param("symbol"))
	if err != nil {
		respondErr(c, err)
		return
	}
	if body.TargetProfitPct != nil {
		p := *body.TargetProfitPct
		if !(p > 0) || p > 1000 {
			respondErr(c, badRequest("target_profit_pct must be between 0 and 1000"))
			return
		}
		limits, err := rules.LimitsFor(st.Pool, models.ModeLong)
		if err != nil {
			respondErr(c, internalErr(err))
			return
		}
		if !limits.AllowsTarget(p) {
			respondErr(c, badRequest(fmt.Sprintf(
				"target_profit_pct must sit inside the pot's band %.1f%%–%.1f%%",
				limits.MinProfitPct, limits.MaxProfitPct)))
			return
		}
	}
	if _, err := st.Pool.Exec("UPDATE instruments SET target_profit_pct = ? WHERE id = ?",
		body.TargetProfitPct, id); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, gin.H{"symbol": c.Param("symbol"), "target_profit_pct": body.TargetProfitPct})
}

type tradeBody struct {
	Side  string  `json:"side"`
	Qty   float64 `json:"qty"`
	Price float64 `json:"price"`
	Note  *string `json:"note"`
	At    *string `json:"at"`
	Mode  *string `json:"mode"`
}

func recordTrade(c *gin.Context, st *AppState) {
	var body tradeBody
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	side, err := trades.ParseSide(body.Side)
	if err != nil {
		respondErr(c, badRequest(err.Error()))
		return
	}
	mode := models.ModeLong
	if body.Mode != nil {
		m, perr := models.ParseMode(*body.Mode)
		if perr != nil {
			respondErr(c, badRequest(perr.Error()))
			return
		}
		mode = m
	}
	id, err := instrumentIDBySymbol(st.Pool, c.Param("symbol"))
	if err != nil {
		respondErr(c, err)
		return
	}
	var at *time.Time
	if body.At != nil {
		when, perr := trades.ParseWhen(*body.At, time.Now().UTC())
		if perr != nil {
			respondErr(c, badRequest(perr.Error()))
			return
		}
		at = &when
	}
	if rerr := trades.Record(st.Pool, id, side, body.Qty, body.Price, body.Note, at, mode); rerr != nil {
		respondErr(c, badRequest(rerr.Error()))
		return
	}
	c.JSON(http.StatusOK, gin.H{"recorded": true, "symbol": c.Param("symbol")})
}

func editTrade(c *gin.Context, st *AppState) {
	id := c.Param("id")
	var body struct {
		Qty   *float64 `json:"qty"`
		Price *float64 `json:"price"`
		At    *string  `json:"at"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	var at *time.Time
	if body.At != nil {
		when, perr := trades.ParseWhen(*body.At, time.Now().UTC())
		if perr != nil {
			respondErr(c, badRequest(perr.Error()))
			return
		}
		at = &when
	}
	if err := trades.Edit(st.Pool, id, body.Qty, body.Price, at); err != nil {
		respondErr(c, badRequest(err.Error()))
		return
	}
	c.JSON(http.StatusOK, gin.H{"id": c.Param("id"), "edited": true})
}

func deleteTrade(c *gin.Context, st *AppState) {
	id := c.Param("id")
	if derr := trades.Delete(st.Pool, id); derr != nil {
		respondErr(c, badRequest(derr.Error()))
		return
	}
	c.JSON(http.StatusOK, gin.H{"id": c.Param("id"), "deleted": true})
}

func postPosition(c *gin.Context, st *AppState) {
	var body struct {
		Symbol  string   `json:"symbol"`
		Qty     float64  `json:"qty"`
		AvgCost float64  `json:"avg_cost"`
		Stop    *float64 `json:"stop"`
		Target  *float64 `json:"target"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	if !(body.Qty > 0) || !(body.AvgCost > 0) {
		respondErr(c, badRequest("qty and avg_cost must be positive"))
		return
	}
	id, err := instrumentIDBySymbol(st.Pool, body.Symbol)
	if err != nil {
		respondErr(c, err)
		return
	}
	cost := rules.Cost(body.Qty, body.AvgCost, models.ClassStock)
	fits, ferr := rules.Fits(st.Pool, cost)
	if ferr != nil {
		respondErr(c, internalErr(ferr))
		return
	}
	if !fits {
		respondErr(c, badRequest("does not fit the budget"))
		return
	}
	tx, err := st.Pool.Begin()
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	now := tstampNow()
	if _, err := tx.Exec(
		`INSERT INTO positions (id, instrument_id, qty, avg_cost, opened_at, stop, target, state)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'open')`,
		ids.New(), id, body.Qty, body.AvgCost, now, body.Stop, body.Target); err != nil {
		tx.Rollback()
		respondErr(c, internalErr(err))
		return
	}
	if err := rules.RecordOn(tx, rules.NewCashEntry{
		Kind: rules.CashBuy, Amount: -cost,
		Reference: fmt.Sprintf("buy %s @ %v", body.Symbol, body.AvgCost),
	}, now, models.ModeLong); err != nil {
		tx.Rollback()
		respondErr(c, internalErr(err))
		return
	}
	if err := tx.Commit(); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	row, _ := ledgerPosition(st.Pool, body.Symbol)
	c.JSON(http.StatusOK, row)
}

func ledgerPosition(pool *sql.DB, symbol string) (*ledger.PositionRow, error) {
	rows, err := ledger.Positions(pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	for i := range rows {
		if rows[i].Symbol == symbol && rows[i].State == "open" {
			return &rows[i], nil
		}
	}
	return nil, nil
}

func patchPosition(c *gin.Context, st *AppState) {
	id := c.Param("id")
	var body struct {
		Qty    *float64 `json:"qty"`
		Stop   *float64 `json:"stop"`
		Target *float64 `json:"target"`
		State  *string  `json:"state"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	var oldQty, avgCost float64
	if err := st.Pool.QueryRow("SELECT qty, avg_cost FROM positions WHERE id = ?", id).
		Scan(&oldQty, &avgCost); err == sql.ErrNoRows {
		respondErr(c, notFound("no position"))
		return
	} else if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	tx, err := st.Pool.Begin()
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if body.Qty != nil {
		if _, err := tx.Exec("UPDATE positions SET qty = ? WHERE id = ?", *body.Qty, id); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
		sold := oldQty - *body.Qty
		if sold > 0 {
			var instrumentID int64
			if err := tx.QueryRow("SELECT instrument_id FROM positions WHERE id = ?", id).Scan(&instrumentID); err != nil {
				tx.Rollback()
				respondErr(c, internalErr(err))
				return
			}
			bucket := ""
			var classRaw string
			if err := tx.QueryRow("SELECT class FROM instruments WHERE id = ?", instrumentID).Scan(&classRaw); err == nil {
				class, _ := models.ParseAssetClass(classRaw)
				bucket = rules.BucketFor(class)
			}
			if err := rules.RecordOn(tx, rules.NewCashEntry{
				Kind: rules.CashSell, Amount: sold * avgCost,
				Reference: fmt.Sprintf("sell position %s", id), Bucket: bucket,
			}, tstampNow(), models.ModeLong); err != nil {
				tx.Rollback()
				respondErr(c, internalErr(err))
				return
			}
		}
	}
	if body.Stop != nil {
		if _, err := tx.Exec("UPDATE positions SET stop = ? WHERE id = ?", *body.Stop, id); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
	}
	if body.Target != nil {
		if _, err := tx.Exec("UPDATE positions SET target = ? WHERE id = ?", *body.Target, id); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
	}
	if body.State != nil {
		if _, err := tx.Exec("UPDATE positions SET state = ? WHERE id = ?", *body.State, id); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
	}
	if err := tx.Commit(); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, gin.H{"id": c.Param("id"), "updated": true})
}

func postIPO(c *gin.Context, st *AppState) {
	var body struct {
		Symbol string  `json:"symbol"`
		Qty    float64 `json:"qty"`
		Price  float64 `json:"price"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	id, err := instrumentIDBySymbol(st.Pool, body.Symbol)
	if err != nil {
		respondErr(c, err)
		return
	}
	if err := rules.ReserveIPO(st.Pool, id, body.Qty, body.Price); err != nil {
		respondErr(c, badRequest(err.Error()))
		return
	}
	c.JSON(http.StatusOK, gin.H{"reserved": true})
}

type approveBody struct {
	Qty   *float64 `json:"qty"`
	Price *float64 `json:"price"`
	At    *string  `json:"at"`
	Fills []struct {
		ID    string   `json:"id"`
		Qty   *float64 `json:"qty"`
		Price *float64 `json:"price"`
	} `json:"fills"`
}

func approveSignal(c *gin.Context, st *AppState) {
	id := c.Param("id")
	var body approveBody
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	var planID sql.NullString
	if err := st.Pool.QueryRow("SELECT plan_id FROM signals WHERE id = ?", id).Scan(&planID); err != nil &&
		err != sql.ErrNoRows {
		respondErr(c, internalErr(err))
		return
	}
	if planID.Valid && planID.String != "" {
		approvePlan(c, st, id, planID.String, body)
		return
	}

	var instrumentID int64
	var action, modeRaw, statusValue string
	var qty, entry, stop, target sql.NullFloat64
	err := st.Pool.QueryRow(
		`SELECT instrument_id, action, qty, entry, stop, target, mode, status
         FROM signals WHERE id = ?`, id,
	).Scan(&instrumentID, &action, &qty, &entry, &stop, &target, &modeRaw, &statusValue)
	if err == sql.ErrNoRows {
		respondErr(c, notFound("no decision "+id))
		return
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if statusValue != "pending" {
		respondErr(c, badRequest("decision "+id+" is already "+statusValue))
		return
	}
	var side trades.Side
	switch action {
	case "BUY":
		side = trades.SideBuy
	case "SELL":
		side = trades.SideSell
	default:
		respondErr(c, badRequest("a "+action+" decision has nothing to record"))
		return
	}
	fillQty := 0.0
	hasQty := false
	if body.Qty != nil && *body.Qty > 0 {
		fillQty, hasQty = *body.Qty, true
	} else if qty.Valid && qty.Float64 > 0 {
		fillQty, hasQty = qty.Float64, true
	}
	if !hasQty {
		respondErr(c, badRequest("quantity is required — the decision did not propose one"))
		return
	}
	fillPrice := 0.0
	hasPrice := false
	if body.Price != nil && *body.Price > 0 {
		fillPrice, hasPrice = *body.Price, true
	} else if entry.Valid && entry.Float64 > 0 {
		fillPrice, hasPrice = entry.Float64, true
	}
	if !hasPrice {
		respondErr(c, badRequest("price is required"))
		return
	}
	mode, perr := models.ParseMode(modeRaw)
	if perr != nil {
		respondErr(c, badRequest(perr.Error()))
		return
	}
	whenRaw := ""
	if body.At != nil {
		whenRaw = *body.At
	}
	when, terr := trades.ParseWhen(whenRaw, time.Now().UTC())
	if terr != nil {
		respondErr(c, badRequest(terr.Error()))
		return
	}
	note := fmt.Sprintf("decision %s", id)
	var already int64
	err = st.Pool.QueryRow("SELECT id FROM trades WHERE note = ? AND instrument_id = ?", note, instrumentID).
		Scan(&already)
	if err == sql.ErrNoRows {
		if rerr := trades.Record(st.Pool, instrumentID, side, fillQty, fillPrice, &note, &when, mode); rerr != nil {
			respondErr(c, badRequest(rerr.Error()))
			return
		}
		if aerr := applyExitPlan(st.Pool, instrumentID, mode, nullFloat(stop), nullFloat(target)); aerr != nil {
			respondErr(c, internalErr(aerr))
			return
		}
	} else if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if _, err := st.Pool.Exec(
		`UPDATE signals SET status = 'approved', status_at = ?, filled_qty = ?, filled_price = ?
         WHERE id = ?`, tstampNow(), fillQty, fillPrice, id); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	slog.Info("decision approved", "id", id, "side", side, "qty", fillQty, "price", fillPrice,
		"recorded", already == 0)
	c.JSON(http.StatusOK, gin.H{
		"id": c.Param("id"), "status": "approved", "filled_qty": fillQty,
		"filled_price": fillPrice, "recorded": already == 0,
	})
}

type planLeg struct {
	ID           string
	InstrumentID int64
	Action       string
	Qty          *float64
	Entry        *float64
	Stop         *float64
	Target       *float64
	Mode         string
}

func approvePlan(c *gin.Context, st *AppState, id, plan string, body approveBody) {
	rows, err := st.Pool.Query(
		`SELECT id, instrument_id, action, qty, entry, stop, target, mode FROM signals
         WHERE plan_id = ? AND status = 'pending'
         ORDER BY CASE action WHEN 'SELL' THEN 0 ELSE 1 END, id`, plan)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	legs := []planLeg{}
	for rows.Next() {
		var leg planLeg
		var qty, entry, stop, target sql.NullFloat64
		if err := rows.Scan(&leg.ID, &leg.InstrumentID, &leg.Action, &qty, &entry, &stop,
			&target, &leg.Mode); err != nil {
			rows.Close()
			respondErr(c, internalErr(err))
			return
		}
		if qty.Valid {
			leg.Qty = &qty.Float64
		}
		if entry.Valid {
			leg.Entry = &entry.Float64
		}
		if stop.Valid {
			leg.Stop = &stop.Float64
		}
		if target.Valid {
			leg.Target = &target.Float64
		}
		legs = append(legs, leg)
	}
	rows.Close()
	if len(legs) == 0 {
		respondErr(c, badRequest(fmt.Sprintf("plan %s has no pending moves left", plan)))
		return
	}
	mode, perr := models.ParseMode(legs[0].Mode)
	if perr != nil {
		respondErr(c, badRequest(perr.Error()))
		return
	}
	type planned struct {
		leg   planLeg
		qty   float64
		price float64
	}
	plan2 := []planned{}
	for _, leg := range legs {
		var q, p *float64
		for _, f := range body.Fills {
			if f.ID == leg.ID {
				q, p = f.Qty, f.Price
				break
			}
		}
		qtyValue := 0.0
		if q != nil && *q > 0 {
			qtyValue = *q
		} else if leg.Qty != nil && *leg.Qty > 0 {
			qtyValue = *leg.Qty
		}
		if qtyValue <= 0 {
			respondErr(c, badRequest(fmt.Sprintf("move %s has no quantity", leg.ID)))
			return
		}
		priceValue := 0.0
		if p != nil && *p > 0 {
			priceValue = *p
		} else if leg.Entry != nil && *leg.Entry > 0 {
			priceValue = *leg.Entry
		}
		if priceValue <= 0 {
			respondErr(c, badRequest(fmt.Sprintf("move %s has no price", leg.ID)))
			return
		}
		plan2 = append(plan2, planned{leg: leg, qty: qtyValue, price: priceValue})
	}
	proceeds, cost := 0.0, 0.0
	for _, p := range plan2 {
		if p.leg.Action == "SELL" {
			proceeds += p.qty * p.price
		} else {
			cost += p.qty * p.price
		}
	}
	var bucket string
	var classRaw string
	err = st.Pool.QueryRow("SELECT class FROM instruments WHERE id = ?", legs[0].InstrumentID).Scan(&classRaw)
	if err == nil {
		class, _ := models.ParseAssetClass(classRaw)
		bucket = rules.BucketFor(class)
	}
	var cash float64
	if bucket != "" {
		cash, err = rules.CashInBucket(st.Pool, mode, bucket)
	} else {
		cash, err = rules.AvailableIn(st.Pool, mode)
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if cost > proceeds+cash+1e-9 {
		respondErr(c, badRequest(fmt.Sprintf(
			"the plan does not balance: buys cost %.2f, its sells free %.2f, cash is %.2f",
			cost, proceeds, cash)))
		return
	}
	whenRaw := ""
	if body.At != nil {
		whenRaw = *body.At
	}
	when, terr := trades.ParseWhen(whenRaw, time.Now().UTC())
	if terr != nil {
		respondErr(c, badRequest(terr.Error()))
		return
	}

	// One transaction for the whole plan: a leg that cannot post undoes the legs before it.
	tx, err := st.Pool.Begin()
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	posted := []string{}
	for _, p := range plan2 {
		side := trades.SideBuy
		if p.leg.Action == "SELL" {
			side = trades.SideSell
		} else if p.leg.Action != "BUY" {
			tx.Rollback()
			respondErr(c, badRequest(fmt.Sprintf("a %s move has nothing to record", p.leg.Action)))
			return
		}
		note := fmt.Sprintf("decision %s", p.leg.ID)
		var already int64
		err := tx.QueryRow("SELECT id FROM trades WHERE note = ? AND instrument_id = ?",
			note, p.leg.InstrumentID).Scan(&already)
		if err == sql.ErrNoRows {
			if rerr := trades.RecordOn(tx, p.leg.InstrumentID, side, p.qty, p.price, &note, &when, mode); rerr != nil {
				tx.Rollback()
				respondErr(c, badRequest(rerr.Error()))
				return
			}
		} else if err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
		if _, err := tx.Exec(
			`UPDATE signals SET status = 'approved', status_at = ?, filled_qty = ?, filled_price = ?
             WHERE id = ?`, tstampNow(), p.qty, p.price, p.leg.ID); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
		if err := applyExitPlanOn(tx, p.leg.InstrumentID, mode, p.leg.Stop, p.leg.Target); err != nil {
			tx.Rollback()
			respondErr(c, internalErr(err))
			return
		}
		posted = append(posted, p.leg.ID)
	}
	if err := tx.Commit(); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	slog.Info("plan approved", "id", id, "plan", plan, "legs", len(posted))
	c.JSON(http.StatusOK, gin.H{"id": c.Param("id"), "plan": plan, "status": "approved", "legs": posted})
}

func applyExitPlan(pool *sql.DB, instrumentID int64, mode models.Mode, stop, target *float64) error {
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err := applyExitPlanOn(tx, instrumentID, mode, stop, target); err != nil {
		return err
	}
	return tx.Commit()
}

func applyExitPlanOn(tx *sql.Tx, instrumentID int64, mode models.Mode, stop, target *float64) error {
	if stop == nil && target == nil {
		return nil
	}
	_, err := tx.Exec(
		`UPDATE positions SET stop = COALESCE(?, stop), target = COALESCE(?, target)
         WHERE id = (SELECT id FROM positions
                     WHERE instrument_id = ? AND mode = ? AND state = 'open'
                     ORDER BY id DESC LIMIT 1)`,
		stop, target, instrumentID, string(mode))
	return err
}

func rejectSignal(c *gin.Context, st *AppState) {
	id := c.Param("id")
	var body struct {
		Reason *string `json:"reason"`
	}
	_ = c.ShouldBindJSON(&body)
	var planID sql.NullString
	if err := st.Pool.QueryRow("SELECT plan_id FROM signals WHERE id = ?", id).Scan(&planID); err != nil &&
		err != sql.ErrNoRows {
		respondErr(c, internalErr(err))
		return
	}
	var reason *string
	if body.Reason != nil {
		trimmed := strings.TrimSpace(*body.Reason)
		if trimmed != "" {
			reason = &trimmed
		}
	}
	// Rejecting one leg of a plan rejects the plan: half its moves gone is not a plan.
	var res sql.Result
	var err error
	if planID.Valid && planID.String != "" {
		res, err = st.Pool.Exec(
			`UPDATE signals SET status = 'rejected', status_at = ?, status_reason = ?
             WHERE status = 'pending' AND (id = ? OR plan_id = ?)`,
			tstampNow(), reason, id, planID.String)
	} else {
		res, err = st.Pool.Exec(
			`UPDATE signals SET status = 'rejected', status_at = ?, status_reason = ?
             WHERE status = 'pending' AND id = ?`, tstampNow(), reason, id)
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if n, _ := res.RowsAffected(); n == 0 {
		respondErr(c, notFound("no pending decision "+id))
		return
	}
	reasonText := ""
	if reason != nil {
		reasonText = *reason
	}
	slog.Info("decision rejected", "id", id, "reason", reasonText)
	c.JSON(http.StatusOK, gin.H{"id": c.Param("id"), "status": "rejected"})
}

func startRun(c *gin.Context, st *AppState) {
	var body struct {
		Prompt string  `json:"prompt"`
		Symbol *string `json:"symbol"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	var prompt runs.Prompt
	var manualKind runs.ManualKind
	switch body.Prompt {
	case "poll":
		prompt, manualKind = runs.Poll, runs.ManualPoll
	case "plan_stock":
		prompt, manualKind = runs.PlanStock, runs.ManualPlanStock
	case "plan_fund":
		prompt, manualKind = runs.PlanFund, runs.ManualPlanFund
	case "plan_trade":
		prompt, manualKind = runs.PlanTrade, runs.ManualPlanTrade
	case "decide":
		prompt, manualKind = runs.Decide, runs.ManualDecide
	default:
		respondErr(c, badRequest(fmt.Sprintf("%q is not something that can be run", body.Prompt)))
		return
	}
	var symbol *string
	if body.Symbol != nil {
		trimmed := strings.TrimSpace(*body.Symbol)
		if trimmed != "" {
			symbol = &trimmed
		}
	}
	if prompt == runs.Decide && symbol == nil {
		respondErr(c, badRequest("decide needs a symbol — it asks about one instrument"))
		return
	}
	if symbol != nil {
		// Fail before writing a run row for an instrument that does not exist.
		if _, err := instrumentIDBySymbol(st.Pool, *symbol); err != nil {
			respondErr(c, err)
			return
		}
	}
	if st.Manual == nil {
		respondErr(c, badRequest("this server is not running its poll loop, so nothing can be started"))
		return
	}
	id, err := runs.Start(st.Pool, prompt, runs.TriggerManual, symbol)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	symbolValue := ""
	if symbol != nil {
		symbolValue = *symbol
	}
	select {
	case st.Manual <- ManualRequest{RunID: id, Cmd: runs.Manual{Kind: manualKind, Symbol: symbolValue}}:
	default:
		_ = runs.Fail(st.Pool, id, "the poll loop is not listening")
		respondErr(c, badRequest("the poll loop is not listening"))
		return
	}
	c.JSON(http.StatusOK, gin.H{"id": id, "status": "running"})
}

func testNotify(c *gin.Context, st *AppState) {
	notifier, err := notify.NtfyFromSettings(st.Pool, st.Cfg.PublicURL)
	if err != nil {
		respondErr(c, badRequest(err.Error()))
		return
	}
	if notifier == nil {
		respondErr(c, badRequest("no topic configured — add one below first"))
		return
	}
	n := notify.Notification{
		Title:     "stock-predictor · test",
		Body:      "If this arrived on your phone, the channel works.",
		Priority:  notify.PriorityDefault,
		DedupeKey: fmt.Sprintf("test-%d", time.Now().Unix()),
	}
	if err := notifier.SendTest(n); err != nil {
		respondErr(c, badRequest("ntfy: "+err.Error()))
		return
	}
	slog.Info("test notification sent")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

func nullFloat(v sql.NullFloat64) *float64 {
	if v.Valid {
		return &v.Float64
	}
	return nil
}
