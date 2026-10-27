import { describe, expect, test } from "vitest";
import type { Dashboard, PositionRow, Settings, SignalRow } from "./api";
import { renderLedger, renderModeTabs, renderRow, renderStrip } from "./pages/ledger";
import { renderSettings, shouldDisableSave } from "./pages/settings";

const position = (over: Partial<PositionRow> = {}): PositionRow => ({
  id: 1,
  symbol: "INFY.NS",
  class: "stock",
  qty: 17,
  avg_cost: 1163,
  ltp: 1200,
  value: 20_400,
  stop: 1104.85,
  target: 1279.3,
  state: "open",
  timing_note: "limit order, intraday or next open",
  ...over,
});

const signal = (over: Partial<SignalRow> = {}): SignalRow => ({
  id: 1,
  symbol: "INFY.NS",
  class: "stock",
  action: "SELL",
  qty: 17,
  entry: 1163,
  stop: 1200,
  target: 1279.3,
  valid_until: "2026-10-28T10:00:00Z",
  timing_note: "limit order, intraday or next open",
  reasons: ["close 1163.00 below recorded stop 1200.00"],
  rationale: "The stop was breached.",
  created_at: "2026-10-27T04:11:00Z",
  ...over,
});

const dashboard = (over: Partial<Dashboard> = {}): Dashboard => ({
  currency: "INR",
  mode: "long",
  budget: 200_000,
  committed: 103_611,
  available: 96_389,
  realised: 1240,
  currency_locked: true,
  positions: [],
  signals: [],
  blocked: [],
  ...over,
});

describe("portfolio strip", () => {
  test("total is invested plus cash, and realised carries its sign", () => {
    const html = renderStrip(dashboard({ positions: [position()] }));
    expect(html).toContain("Total value");
    expect(html).toContain("1,16,789"); // 20,400 invested + 96,389 cash
    expect(html).toContain("Realised");
    expect(html).toContain("+1,240");
  });

  test("a negative realised figure is shown as a loss, signed", () => {
    const html = renderStrip(dashboard({ realised: -500 }));
    expect(html).toContain("-500");
    expect(html).toContain('class="num down"');
  });
});

describe("blotter row", () => {
  test("carries the symbol for navigation, the numbers, and the signal stamp", () => {
    const html = renderRow(dashboard({ positions: [position()], signals: [signal()] }), position());
    expect(html).toContain('data-symbol="INFY.NS"');
    expect(html).toContain("1,163.00"); // avg
    expect(html).toContain("1,200.00"); // ltp
    expect(html).toContain("20,400"); // value
    expect(html).toContain('data-stamp="sell"');
    expect(html).toContain('role="link"');
  });

  test("a fund is labelled so it is never mistaken for an intraday price", () => {
    const mf = position({ symbol: "AXISELSS", class: "mutual_fund", stop: null, ltp: 105.36, value: 25_000 });
    const html = renderRow(dashboard({ positions: [mf] }), mf);
    expect(html).toContain('data-stamp="estimate"');
    expect(html).toContain("MF");
  });

  test("no signal renders an em dash, not an empty cell", () => {
    const html = renderRow(dashboard(), position());
    expect(html).toContain("—");
    expect(html).not.toContain('data-stamp="buy"');
  });
});

describe("ledger", () => {
  test("empty portfolio explains what to do next", () => {
    const html = renderLedger(dashboard(), []);
    expect(html).toContain("Nothing held yet");
    expect(html).toContain("No trades recorded yet");
    expect(html).not.toContain("spinner");
  });

  test("recent trades render with their side and date", () => {
    const html = renderLedger(dashboard(), [
      {
        id: 1,
        symbol: "INFY.NS",
        side: "buy",
        qty: 17,
        price: 1163,
        realised: null,
        traded_at: "2026-10-20T04:00:00Z",
        note: null,
        market_close: null,
        mode: "long",
      },
    ]);
    expect(html).toContain("INFY.NS 17 @ 1,163.00");
    expect(html).toContain('data-stamp="buy"');
  });

  test("search input is present and labelled", () => {
    const html = renderLedger(dashboard(), []);
    expect(html).toContain('id="search"');
    expect(html).toContain("Search stocks and mutual funds");
  });
});

describe("settings", () => {
  const settings = (over: Partial<Settings> = {}): Settings => ({
    budget: 100_000,
    committed: 0,
    available: 100_000,
    currency: "INR",
    currency_locked: false,
    deepseek_configured: false,
    trading_budget: 0,
    trading_available: 0,
    ...over,
  });

  test("disables save when the budget is below committed", () => {
    const html = renderSettings(settings({ committed: 142_000, available: -42_000 }));
    expect(html).toContain("Committed is");
    expect(html).toContain("disabled");
  });

  test("a configured key is masked and offers to keep it", () => {
    const html = renderSettings(settings({ deepseek_configured: true }));
    expect(html).toContain('type="password"');
    expect(html).toContain("leave blank to keep");
    expect(html).toContain("never shown again");
  });

  test("disables currency once locked, with the reason", () => {
    const html = renderSettings(settings({ currency_locked: true }));
    expect(html).toContain("Currency is fixed once the ledger has entries");
  });

  test("save guard is exact at the boundary", () => {
    expect(shouldDisableSave(141_999, 142_000)).toBe(true);
    expect(shouldDisableSave(142_000, 142_000)).toBe(false);
    expect(shouldDisableSave(NaN, 0)).toBe(true);
  });
});

describe("trade form dates", () => {
  test("offers a date field defaulting to today, so old fills can be backfilled", async () => {
    const { renderTradeForm, today } = await import("./pages/instrument");
    const html = renderTradeForm("INFY.NS", { qty: 17 }, new Date("2026-10-27T12:00:00Z"));
    expect(html).toContain('name="at"');
    expect(html).toContain('type="date"');
    expect(html).toContain(`value="${today(new Date("2026-10-27T12:00:00Z"))}"`);
    expect(html).toContain("backfill an older fill");
  });

  test("caps the date at today — a fill cannot be in the future", async () => {
    const { renderTradeForm } = await import("./pages/instrument");
    const html = renderTradeForm("INFY.NS", null, new Date("2026-10-27T12:00:00Z"));
    expect(html).toContain('max="2026-10-27"');
  });

  test("a buy is never capped by the current holding", async () => {
    const { renderTradeForm } = await import("./pages/instrument");
    const html = renderTradeForm("INFY.NS", { qty: 2 });
    // Regression: a static max="2" made the browser refuse "buy 10" of something you hold 2 of.
    expect(html).not.toContain('max="2"');
    expect(html).toContain('data-held="2"');
    expect(html).toContain("held 2");
  });

  test("sell is capped by what is held, buy is not", async () => {
    const { qtyLimit } = await import("./pages/instrument");
    expect(qtyLimit("sell", 17)).toBe(17);
    expect(qtyLimit("buy", 17)).toBeNull();
    expect(qtyLimit("sell", null)).toBeNull();
  });
});

describe("history dates", () => {
  test("includes the year once the fill is not from the current one", async () => {
    const { histDate } = await import("./format");
    const now = new Date("2026-10-27T12:00:00Z");
    expect(histDate("2026-08-20T10:00:00Z", now)).not.toContain("2026");
    expect(histDate("2025-08-20T10:00:00Z", now)).toContain("2025");
    expect(histDate(null, now)).toBe("—");
  });
});

describe("fills against the market", () => {
  test("shows the close on the fill date and the difference", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const analysis = {
      instrument: { id: 1, symbol: "INFY.NS", class: "stock", name: "Infosys", currency: "INR", timing_note: "limit order" },
      bars: [{ ts: "2026-10-27T03:45:00Z", close: 1163 }],
      indicators: null,
      checks: [],
      position: null,
      headlines: [],
    };
    const detail = { instrument: analysis.instrument, headlines: [], signals: [] };
    const list = [
      {
        id: 1, symbol: "INFY.NS", side: "buy" as const, qty: 17, price: 1200,
        realised: null, traded_at: "2026-10-27T10:00:00Z", note: null, market_close: 1163,
      },
    ];
    const html = renderInstrument(analysis as never, detail as never, list);
    expect(html).toContain("Close that day");
    expect(html).toContain("1,163.00");
    expect(html).toContain("+3.18%"); // paid 1200 against a 1163 close
  });

  test("a missing close is an em dash, never a zero", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const analysis = {
      instrument: { id: 1, symbol: "X.NS", class: "stock", name: "X", currency: "INR", timing_note: "" },
      bars: [], indicators: null, checks: [], position: null, headlines: [],
    };
    const list = [
      { id: 1, symbol: "X.NS", side: "buy" as const, qty: 1, price: 100, realised: null,
        traded_at: "2019-01-01T10:00:00Z", note: null, market_close: null },
    ];
    const html = renderInstrument(analysis as never, { instrument: analysis.instrument, headlines: [], signals: [] } as never, list);
    expect(html).toContain("—");
    expect(html).not.toContain("0.00%");
  });
});

describe("two pots in the UI", () => {
  test("mode tabs mark the current pot and link to the other", () => {
    const html = renderModeTabs("trade");
    expect(html).toContain('href="/?mode=long"');
    expect(html).toContain('href="/?mode=trade"');
    expect(html).toContain('aria-current="page">Trading');
  });

  test("only trades from the current pot are listed", () => {
    const trade = (id: number, mode: "long" | "trade") => ({
      id, symbol: "INFY.NS", side: "buy" as const, qty: 1, price: 100,
      realised: null, traded_at: "2026-10-27T10:00:00Z", note: null,
      market_close: null, mode,
    });
    const html = renderLedger(
      dashboard({ mode: "trade", positions: [position()] }),
      [trade(1, "long"), trade(2, "trade")],
    );
    expect(html).toContain("Trading book");
    expect(html.match(/class="hist"/g)?.length).toBe(1);
  });
});

describe("settings, second pot", () => {
  test("offers a trading budget and says the pots are separate", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({
      budget: 500_000, committed: 0, available: 500_000, currency: "INR",
      currency_locked: false, deepseek_configured: false,
      trading_budget: 100_000, trading_available: 80_000,
    });
    expect(html).toContain('name="trading_budget"');
    expect(html).toContain("80,000");
    expect(html).toContain("never draws on the long-term budget");
  });
});
