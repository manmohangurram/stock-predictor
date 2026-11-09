import { describe, expect, test } from "vitest";
import type { CallRow, Dashboard, PositionRow, Settings, SignalRow } from "./api";
import { renderLedger, renderModeTabs, renderRow, renderStrip } from "./pages/ledger";
import { renderSettings, shouldDisableSave } from "./pages/settings";
import type { Limits } from "./api";

const limits = (over: Partial<Limits> = {}): Limits => ({
  min_profit_pct: 5,
  max_profit_pct: 50,
  min_trade_amount: 500,
  max_trade_amount: 50_000,
  max_instrument_pct: 25,
  ...over,
});

const position = (over: Partial<PositionRow> = {}): PositionRow => ({
  id: 1,
  symbol: "INFY.NS",
  name: "Infosys",
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
  status: "pending",
  status_reason: null,
  filled_qty: null,
  filled_price: null,
  qty: 17,
  confidence: null,
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
  recent_trades: [],
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

  test("a fund row says units at NAV, not an intraday price", () => {
    const mf = position({ symbol: "AXISELSS", class: "mutual_fund", stop: null, ltp: 105.36, value: 25_000 });
    const html = renderRow(dashboard({ positions: [mf] }), mf);
    expect(html).toContain("units at NAV");
    // No MF badge any more: funds have their own table, so the label is redundant.
    expect(html).not.toContain('data-stamp="estimate"');
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
    expect(html).toContain("No stocks yet");
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
    ntfy_url: "https://ntfy.sh",
    ntfy_topic: "",
    ntfy_token_configured: false,
    llm_provider: "deepseek",
    llm_model: "deepseek-chat",
    llm_base_url: "https://api.deepseek.com",
    llm_key_configured: false,
    limits_long: limits(),
    limits_trade: limits(),
    runtime: {
      poll_secs: 900,
      off_hours_secs: 7200,
      prompt_window_hours: 4,
      min_price_move: 2,
      max_silence_secs: 3600,
      daily_cap: 200,
      proxy_configured: false,
      proxy_host: "",
      proxy_user: null,
      proxy_pass_configured: false,
      proxy_bypass: null,
    },
    ...over,
  });

  test("six tabs render, and only the active one shows its fields", () => {
    const html = renderSettings(settings(), "cadence");
    for (const id of ["money", "risk", "model", "notifications", "cadence", "network"]) {
      expect(html).toContain(`id="tab-${id}"`);
    }
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('data-tab="cadence"');
    expect(html).toContain('id="poll_secs"');
    // A Money field must not be rendered on the Cadence tab.
    expect(html).not.toContain('id="budget"');
  });

  test("each tab carries its own stamp, never a page-wide save", () => {
    for (const tab of ["money", "risk", "model", "notifications", "cadence", "network"] as const) {
      const html = renderSettings(settings(), tab);
      expect(html).toContain(">Stamp<");
      expect(html).toContain('id="revert"');
    }
  });

  test("the budget tab warns when committed is above the ceiling", () => {
    const html = renderSettings(settings({ committed: 142_000, available: -42_000 }), "money");
    expect(html).toContain("free this month");
  });

  test("a configured key is masked and offers to keep it", () => {
    const html = renderSettings(settings({ llm_key_configured: true }), "model");
    expect(html).toContain('type="password"');
    expect(html).toContain("leave blank to keep");
    expect(html).toContain("never shown again");
  });

  test("disables currency once locked, with the reason", () => {
    const html = renderSettings(settings({ currency_locked: true }), "money");
    expect(html).toContain("Currency is fixed once the ledger has entries");
  });

  test("risk limits are one comparison table, two pots side by side", () => {
    const html = renderSettings(settings(), "risk");
    expect(html).toContain("limits-table");
    expect(html).toContain("id=\"long_min_profit_pct\"");
    expect(html).toContain("id=\"trade_max_profit_pct\"");
    expect(html).toContain("band's maximum always wins");
  });

  test("cadence shows the six knobs with their units", () => {
    const html = renderSettings(settings(), "cadence");
    for (const name of ["poll_secs", "off_hours_secs", "prompt_window_hours", "daily_cap", "min_price_move", "max_silence_secs"]) {
      expect(html).toContain(`id="${name}"`);
    }
    expect(html).toContain("no restart");
  });

  test("network never prints the proxy URL, only where it points", () => {
    const html = renderSettings(
      settings({
        runtime: { ...settings().runtime, proxy_configured: true, proxy_host: "10.0.0.2:3128" },
      }),
      "network",
    );
    expect(html).toContain("10.0.0.2:3128");
    expect(html).toContain("HTTP_PROXY");
    expect(html).toContain('id="test-proxy"');
    expect(html).toContain('id="proxy_user"');
    expect(html).toContain('id="proxy_pass"');
    expect(html).toContain('type="password"');
  });

  test("a configured proxy password is masked and kept when left blank", () => {
    const html = renderSettings(
      settings({
        runtime: {
          ...settings().runtime,
          proxy_configured: true,
          proxy_user: "mohan",
          proxy_pass_configured: true,
        },
      }),
      "network",
    );
    expect(html).toContain("leave blank to keep");
    expect(html).toContain("never shown again");
    expect(html).toContain('value="mohan"');
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
    // The backfill hint moved to the help line under the button.
    expect(html).toContain("A buy is limited by cash");
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
    expect(html).toContain('aria-current="page">Short-term');
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
    expect(html).toContain("Short-term book");
    expect(html.match(/class="feed-row"/g)?.length).toBe(1);
  });
});

describe("settings, second pot", () => {
  test("offers a trading budget and says the pots are separate", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({
      budget: 500_000, committed: 0, available: 500_000, currency: "INR",
      currency_locked: false, deepseek_configured: false,
      trading_budget: 100_000, trading_available: 80_000,
      ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
      llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
      llm_key_configured: false, limits_long: limits(), limits_trade: limits(),
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      max_silence_secs: 3600, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null,
    },
    });
    expect(html).toContain('name="trading_budget"');
    expect(html).toContain("80,000");
    expect(html).toContain("never share money");
  });
});

describe("confidence", () => {
  test("shape encodes the level, so colour is never the only signal", async () => {
    const { renderConfidence } = await import("./components/confidence");
    const high = renderConfidence("high");
    expect(high).toContain('data-level="high"');
    expect(high.match(/data-on="true"/g)?.length).toBe(3);
    expect(high).toContain('aria-label="Confidence: high"');

    const medium = renderConfidence("medium");
    expect(medium.match(/data-on="true"/g)?.length).toBe(2);

    const low = renderConfidence("low");
    expect(low.match(/data-on="true"/g)?.length).toBe(1);
  });

  test("no score draws nothing at all — never an empty gauge", async () => {
    const { renderConfidence, confidenceLabel, hasConfidence } = await import(
      "./components/confidence"
    );
    for (const missing of [null, undefined, "", "certain"]) {
      expect(renderConfidence(missing as never)).toBe("");
      expect(confidenceLabel(missing as never)).toBe("");
      expect(hasConfidence(missing as never)).toBe(false);
    }
  });

  test("a rules-derived signal says so in words, not as a score", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const analysis = {
      instrument: { id: 1, symbol: "X.NS", class: "stock", name: "X", currency: "INR", timing_note: "" },
      bars: [], indicators: null, checks: [], position: null, headlines: [],
    };
    const detail = {
      instrument: analysis.instrument,
      headlines: [],
      signals: [
        {
          id: 1, symbol: "X.NS", class: "stock", action: "SELL", qty: 1, confidence: null,
          entry: 100, stop: 110, target: null, valid_until: "2026-10-28T10:00:00Z",
          timing_note: "", reasons: ["stop breached"], rationale: null,
          created_at: "2026-10-27T10:00:00Z",
          status: "pending", status_reason: null, filled_qty: null, filled_price: null,
        },
      ],
    };
    const html = renderInstrument(analysis as never, detail as never, []);
    expect(html).not.toContain('class="conf"');
    expect(html).toContain("rules-derived, no model score");
  });

  test("the blotter row shows it only when the model gave a score", async () => {
    const { renderRow } = await import("./pages/ledger");
    const withScore = renderRow(
      dashboard({ positions: [position()], signals: [signal({ confidence: "high" })] }),
      position(),
    );
    expect(withScore).toContain('aria-label="Confidence: high"');

    const without = renderRow(
      dashboard({ positions: [position()], signals: [signal({ confidence: null })] }),
      position(),
    );
    expect(without).not.toContain('class="conf"');
    expect(without).toContain('data-stamp="sell"');
  });
});

describe("risk and profit expectation", () => {
  const base = {
    id: 1, symbol: "INFY.NS", class: "stock" as const, action: "BUY", qty: 10,
    entry: 100, stop: 90, target: 120, confidence: "high",
    valid_until: "2026-10-28T10:00:00Z", timing_note: "", reasons: [], rationale: null,
    created_at: "2026-10-27T10:00:00Z",
  };

  test("risk is computed from the validated levels, not from the model", async () => {
    const { riskOf } = await import("./components/risk");
    const r = riskOf(base as never, null)!;
    expect(r.amount).toBe(100); // 10 units x 10 of risk
    expect(r.perUnit).toBe(10);
    expect(r.rewardRisk).toBeCloseTo(2.0); // 20 reward / 10 risk
    expect(r.profitPct).toBeCloseTo(20.0);
  });

  test("a target below the owner's expectation is flagged, not hidden", async () => {
    const { riskOf } = await import("./components/risk");
    expect(riskOf(base as never, 15)!.meetsTarget).toBe(true);
    expect(riskOf(base as never, 25)!.meetsTarget).toBe(false);
    expect(riskOf(base as never, null)!.meetsTarget).toBeNull();
  });

  test("no stop means no risk figure rather than a flattering one", async () => {
    const { riskOf } = await import("./components/risk");
    expect(riskOf({ ...base, stop: null } as never, null)).toBeNull();
    expect(riskOf({ ...base, stop: 105 } as never, null)).toBeNull(); // stop above entry
  });

  test("the decision card shows the risk block and the expectation", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const instrument = { id: 1, symbol: "INFY.NS", class: "stock", name: "Infosys", currency: "INR", timing_note: "", target_profit_pct: 15 };
    const analysis = { instrument, bars: [], indicators: null, checks: [], position: null, headlines: [] };
    const detail = { instrument, headlines: [], signals: [base] };
    const html = renderInstrument(analysis as never, detail as never, []);
    expect(html).toContain("At risk");
    expect(html).toContain("Reward : risk");
    expect(html).toContain("meets 15%");
    expect(html).toContain('name="pct"');
  });
});

describe("one request per page", () => {
  test("the ledger gets its recent fills from the dashboard payload", async () => {
    const { renderLedger } = await import("./pages/ledger");
    const d = {
      ...dashboard({ positions: [position()] }),
      recent_trades: [
        {
          id: 1, symbol: "INFY.NS", side: "buy" as const, qty: 17, price: 1163,
          realised: null, traded_at: "2026-10-20T04:00:00Z", note: null,
          market_close: null, mode: "long" as const,
        },
      ],
    };
    const html = renderLedger(d, d.recent_trades);
    expect(html).toContain("INFY.NS 17 @ 1,163.00");
    expect(html).toContain("Recent fills");
  });

  test("the instrument page renders from the analysis payload alone", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const instrument = {
      id: 1, symbol: "INFY.NS", class: "stock", name: "Infosys", currency: "INR",
      timing_note: "", target_profit_pct: null,
    };
    const payload = {
      instrument,
      bars: [], indicators: null, checks: [], position: null, headlines: [],
      signals: [
        {
          id: 1, symbol: "INFY.NS", class: "stock", action: "SELL", qty: 1, confidence: "high",
          entry: 100, stop: 110, target: null, valid_until: "2026-10-28T10:00:00Z",
          timing_note: "", reasons: [], rationale: null, created_at: "2026-10-27T10:00:00Z",
        },
      ],
      trades: [
        {
          id: 1, symbol: "INFY.NS", side: "buy", qty: 1, price: 100, realised: null,
          traded_at: "2026-10-20T04:00:00Z", note: null, market_close: 101, mode: "long",
        },
      ],
    };
    const html = renderInstrument(payload as never, payload as never, payload.trades as never);
    expect(html).toContain("SELL");
    expect(html).toContain("Close that day");
  });
});

describe("ntfy settings", () => {
  const base = {
    budget: 100_000, committed: 0, available: 100_000, currency: "INR", currency_locked: false,
    deepseek_configured: false, trading_budget: 0, trading_available: 0,
    ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
    llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
    llm_key_configured: false, limits_long: limits(), limits_trade: limits(),
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      max_silence_secs: 3600, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null,
    },
  };

  test("offers topic, server and token, plus a way to prove it", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({ ...base, ntfy_topic: "stock-predicter-abc123" }, "notifications");
    expect(html).toContain('name="ntfy_topic"');
    expect(html).toContain('value="stock-predicter-abc123"');
    expect(html).toContain('name="ntfy_url"');
    expect(html).toContain('name="ntfy_token"');
    // The deep-link host is PUBLIC_URL in the environment now, not a settings field.
    expect(html).not.toContain('name="public_url"');
    expect(html).toContain("Send a test notification");
    expect(html).toContain("Anyone who knows the name can read it");
  });

  test("a configured token is masked and never echoed", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({ ...base, ntfy_token_configured: true }, "notifications");
    expect(html).toContain('type="password"');
    expect(html).toContain("leave blank to keep");
    expect(html).toContain("never shown again");
  });
});

describe("model provider settings", () => {
  const base = {
    budget: 100_000, committed: 0, available: 100_000, currency: "INR", currency_locked: false,
    deepseek_configured: false, trading_budget: 0, trading_available: 0,
    ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
    llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
    llm_key_configured: false, limits_long: limits(), limits_trade: limits(),
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      max_silence_secs: 3600, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null,
    },
  };

  test("offers every provider and shows which is selected", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({ ...base, llm_provider: "gemini" }, "model");
    for (const p of ["deepseek", "openai", "gemini", "custom"]) {
      expect(html).toContain(`value="${p}"`);
    }
    expect(html).toContain('<option value="gemini" selected>');
    expect(html).toContain('name="llm_model"');
    expect(html).toContain('name="llm_base_url"');
  });

  test("the key field is one box for whichever provider is chosen", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({ ...base, llm_provider: "openai", llm_key_configured: true }, "model");
    expect(html).toContain('name="llm_key"');
    expect(html).toContain("Configured. The key is never shown again");
  });
});

describe("per-pot limits", () => {
  test("both pots get their own five numbers", async () => {
    const { renderSettings } = await import("./pages/settings");
    const s = {
      budget: 500_000, committed: 0, available: 500_000, currency: "INR", currency_locked: false,
      deepseek_configured: false, trading_budget: 50_000, trading_available: 50_000,
      ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
      llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
      llm_key_configured: false,
      limits_long: limits({ max_trade_amount: 25_000 }),
      limits_trade: limits({ max_trade_amount: 5_000, max_instrument_pct: 10 }),
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      max_silence_secs: 3600, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null,
    },
    };
    const html = renderSettings(s, "risk");
    for (const mode of ["long", "trade"]) {
      for (const name of ["min_profit_pct", "max_profit_pct", "min_trade_amount", "max_trade_amount", "max_instrument_pct"]) {
        expect(html).toContain(`name="${mode}_${name}"`);
      }
    }
    expect(html).toContain("Long-term limits");
    expect(html).toContain("Short-term limits");
    // different values per pot
    expect(html).toContain('id="long_max_trade_amount"');
    expect(html).toContain('value="25000"');
    expect(html).toContain('value="5000"');
  });

  test("the take-profit band is explained in plain words", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings({
      budget: 1000, committed: 0, available: 1000, currency: "INR", currency_locked: false,
      deepseek_configured: false, trading_budget: 0, trading_available: 0,
      ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
      llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "", llm_key_configured: false,
      limits_long: limits(), limits_trade: limits(),
      runtime: {
        poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
        max_silence_secs: 3600, daily_cap: 200, proxy_configured: false, proxy_host: "",
        proxy_user: null, proxy_pass_configured: false, proxy_bypass: null,
      },
    }, "risk");
    expect(html).toContain("A take-profit below this is rejected");
    expect(html).toContain("no fantasy numbers");
    expect(html).toContain("not worth its fees");
  });
});

describe("mutual funds in the trading pot", () => {
  test("an MF is offered the long-term pot only", async () => {
    const { renderTradeForm } = await import("./pages/instrument");
    const html = renderTradeForm("AXISELSS", null, new Date("2026-11-08T12:00:00Z"), "mutual_fund");
    // A fund gets no pot selector at all: the pot is a hidden input fixed to long.
    expect(html).toContain("Long-term only");
    expect(html).not.toContain('<option value="trade">');
    expect(html).toContain('name="mode" value="long"');
  });

  test("a stock still gets both pots", async () => {
    const { renderTradeForm } = await import("./pages/instrument");
    const html = renderTradeForm("INFY.NS", null, new Date("2026-11-08T12:00:00Z"), "stock");
    expect(html).toContain('<option value="trade">Short-term</option>');
  });
});

describe("cadence help text", () => {
  test("seconds read as a person would say them", async () => {
    const { humanDuration } = await import("./pages/settings");
    expect(humanDuration(900)).toBe("15 minutes");
    expect(humanDuration(7200)).toBe("2 hours");
    expect(humanDuration(3600)).toBe("1 hour");
    expect(humanDuration(5400)).toBe("1 h 30 min");
    expect(humanDuration(45)).toBe("45 seconds");
    expect(humanDuration(0)).toBe("—");
  });

  test("the help line carries the value so it can be recomputed live", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings(
      {
        budget: 1000, committed: 0, available: 1000, currency: "INR", currency_locked: false,
        deepseek_configured: false, trading_budget: 0, trading_available: 0,
        ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
        llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "", llm_key_configured: false,
        limits_long: limits(), limits_trade: limits(),
        runtime: {
          poll_secs: 7200, off_hours_secs: 900, prompt_window_hours: 4, min_price_move: 2,
          max_silence_secs: 3600, daily_cap: 200, proxy_configured: false, proxy_host: "",
          proxy_user: null, proxy_pass_configured: false, proxy_bypass: null,
        },
      },
      "cadence",
    );
    // The text follows the stored value, not a hardcoded "15 minutes".
    expect(html).toContain("wakes every 2 hours");
    expect(html).toContain("wakes every 15 minutes");
    expect(html).toContain('data-seconds="poll_secs"');
    expect(html).toContain('data-template="wakes every {v}"');
  });
});

describe("reading a stored call", () => {
  const request = JSON.stringify({
    model: "deepseek-chat",
    messages: [
      { role: "system", content: "You are an analyst." },
      { role: "user", content: "INFY.NS close 1167.40" },
    ],
    tools: [{ type: "function", function: { name: "need_more" } }],
  });

  test("the prompt is shown role by role, not as one blob", async () => {
    const { promptBlocks } = await import("./components/call-viewer");
    const blocks = promptBlocks(request);
    expect(blocks.map((b) => b.label)).toEqual(["system", "user"]);
    expect(blocks[1].text).toContain("close 1167.40");
  });

  test("the reply is the message content, not the envelope", async () => {
    const { replyText } = await import("./components/call-viewer");
    const openai = JSON.stringify({ choices: [{ message: { content: "PASS\nno entry" } }] });
    expect(replyText(openai)).toBe("PASS\nno entry");
    const gemini = JSON.stringify({ candidates: [{ content: { parts: [{ text: "BUY" }] } }] });
    expect(replyText(gemini)).toBe("BUY");
    // A body that is not JSON still shows something rather than nothing.
    expect(replyText("upstream timed out")).toBe("upstream timed out");
  });

  test("declared tools are named, so the escalation is visible", async () => {
    const { declaredTools } = await import("./components/call-viewer");
    expect(declaredTools(request)).toEqual(["need_more"]);
    expect(declaredTools(null)).toEqual([]);
  });
});

describe("reading a reply as a decision", () => {
  test("a decide body becomes a verdict with its numbers", async () => {
    const { parseReply } = await import("./components/call-viewer");
    const r = parseReply(
      JSON.stringify({
        action: "decide",
        signal: {
          action: "SELL", qty: 24, entry: 1171.6, stop: 1080, target: 1420,
          confidence: "medium", rationale: "structure broke", cites: [1, 2],
        },
      }),
    );
    expect(r.verdict).toBe("SELL");
    expect(r.qty).toBe(24);
    expect(r.entry).toBe(1171.6);
    expect(r.confidence).toBe("medium");
    expect(r.cites).toEqual([1, 2]);
  });

  test("a pass carries its reason and no numbers", async () => {
    const { parseReply } = await import("./components/call-viewer");
    const r = parseReply(JSON.stringify({ action: "pass", why: "nothing changed" }));
    expect(r.verdict).toBe("PASS");
    expect(r.why).toBe("nothing changed");
    expect(r.entry).toBeNull();
  });

  test("need_more says what it wants", async () => {
    const { parseReply } = await import("./components/call-viewer");
    const r = parseReply(
      JSON.stringify({ action: "need_more", need_more: { headlines: [3, 7], price_history: true } }),
    );
    expect(r.verdict).toBe("NEED_MORE");
    expect(r.wantsHeadlines).toEqual([3, 7]);
    expect(r.wantsHistory).toBe(true);
  });

  test("a body that is not JSON is not dressed up as a verdict", async () => {
    const { parseReply } = await import("./components/call-viewer");
    expect(parseReply("upstream timeout").verdict).toBeNull();
  });
});

describe("decisions on the instrument page", () => {
  const base = {
    instrument: {
      id: 1, symbol: "INFY.NS", class: "stock", name: "Infosys", currency: "INR",
      timing_note: "limit order", target_profit_pct: null,
    },
    bars: [], indicators: null, checks: [], position: null, headlines: [], trades: [],
  };
  const signal = (over: Record<string, unknown>) => ({
    id: 7, symbol: "INFY.NS", class: "stock", action: "SELL", qty: 24, confidence: "medium",
    entry: 1171.6, stop: 1080, target: 1420, valid_until: "2026-11-09T00:00:00Z",
    timing_note: "", reasons: ["target reached"], rationale: "take it",
    created_at: "2026-11-08T10:00:00Z", status: "pending", status_reason: null,
    filled_qty: null, filled_price: null, ...over,
  });

  test("a pending decision carries its own approve and reject", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const html = renderInstrument(
      base as never,
      { ...base, signals: [signal({})] } as never,
      [],
    );
    expect(html).toContain('data-approve="7"');
    expect(html).toContain('data-approve-form="7"');
    expect(html).toContain('value="1171.6"'); // pre-filled from the decision
    expect(html).toContain('value="24"');
  });

  test("an approved decision shows its status and no actions", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const html = renderInstrument(
      base as never,
      { ...base, signals: [signal({ status: "approved", filled_qty: 8, filled_price: 1170 })] } as never,
      [],
    );
    expect(html).toContain(">approved<");
    expect(html).not.toContain('data-approve="7"');
  });
});

describe("removing an instrument", () => {
  const base = {
    instrument: {
      id: 1, symbol: "TCS.NS", class: "stock", name: "TCS", currency: "INR",
      timing_note: "limit order", target_profit_pct: null,
    },
    bars: [], indicators: null, checks: [], position: null, headlines: [], signals: [], trades: [],
  };

  test("an unheld instrument offers a delete", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const html = renderInstrument(base as never, base as never, []);
    expect(html).toContain('id="archive-instrument"');
    expect(html).toContain("Archive this instrument");
    expect(html).toContain("fills and decisions stay");
  });

  test("a held instrument refuses instead of offering it", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const held = {
      ...base,
      position: {
        id: 1, symbol: "TCS.NS", name: "TCS", class: "stock", qty: 4, avg_cost: 100,
        ltp: 110, value: 440, stop: 90, target: 130, state: "open", timing_note: "",
      },
    };
    const html = renderInstrument(held as never, held as never, []);
    expect(html).toContain("Sell it before archiving");
    expect(html).toContain("cannot be archived");
  });
});

describe("the api client speaks JSON", () => {
  test("approve and reject send a JSON content-type", async () => {
    const calls: RequestInit[] = [];
    const original = globalThis.fetch;
    // A stub, so this catches the 415 before it reaches a server: axum's Json extractor
    // refuses a body without the header.
    globalThis.fetch = (async (_url: string, init?: RequestInit) => {
      calls.push(init ?? {});
      return new Response(JSON.stringify({ ok: true, id: 1, status: "approved" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;
    try {
      const { api } = await import("./api");
      await api.approveSignal(1, { qty: 2, price: 100 });
      await api.rejectSignal(1, "no");
      expect(calls).toHaveLength(2);
      for (const c of calls) {
        const headers = (c.headers ?? {}) as Record<string, string>;
        expect(headers["content-type"]).toBe("application/json");
        expect(c.method).toBe("POST");
      }
    } finally {
      globalThis.fetch = original;
    }
  });
});

describe("decisions filter on the overview", () => {
  const base = {
    instrument: {
      id: 1, symbol: "INFY.NS", class: "stock", name: "Infosys", currency: "INR",
      timing_note: "limit order", target_profit_pct: null,
    },
    bars: [], indicators: null, checks: [], position: null, headlines: [], trades: [],
  };
  const sig = (id: number, status: string) => ({
    id, symbol: "INFY.NS", class: "stock", action: "SELL", qty: 1, confidence: null,
    entry: 100, stop: 90, target: 120, valid_until: null, timing_note: "", reasons: [],
    rationale: null, created_at: "2026-11-08T10:00:00Z", status,
    status_reason: null, filled_qty: null, filled_price: null,
  });

  test("the tabs offer every status", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const html = renderInstrument(
      base as never,
      { ...base, signals: [sig(1, "pending")] } as never,
      [],
    );
    for (const f of ["pending", "approved", "rejected", "all"]) {
      expect(html).toContain(`?decisions=${f}`);
    }
  });

  test("the default shows the whole record, and the reason a decision was rejected", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const rejected = { ...sig(2, "rejected"), status_reason: "not at this price" };
    const html = renderInstrument(
      base as never,
      { ...base, signals: [sig(1, "pending"), rejected] } as never,
      [],
    );
    // Both rows present: the overview is the record of this stock, not just a to-do list.
    expect(html).toContain('data-approve="1"');
    expect(html).toContain(">rejected<");
  });
});

describe("audit", () => {
  test("a blocked call keeps the row short and explains itself inside View", async () => {
    const { renderCalls, renderExchangeDetail } = await import("./pages/calls");
    const row: CallRow = {
      id: 7,
      ts: "2026-11-09T05:34:20Z",
      endpoint: "chat",
      purpose: "decide INFY.NS",
      round: 1,
      prompt_tokens: 10,
      completion_tokens: 2,
      validator_verdict: "pass",
      error: null,
      trigger: "price moved 1,150.20 → 1,171.60",
      decision_id: 4,
      decision_status: "pending",
      decision_action: "BLOCKED",
      decision_reason: "insufficient_cash: cost 1148700.00, available 200000.00",
      instrument_name: "Infosys",
      instrument_symbol: "INFY.NS",
    };
    const list = renderCalls([row]);
    expect(list).toContain("Infosys");
    expect(list).toContain("blocked");
    // The reason must not render in the row — as visible text it pushed the table off the page.
    // A tooltip is fine.
    expect(list).not.toContain('<div class="sub">insufficient');
    expect(list).toContain('title="insufficient_cash');

    const detail = renderExchangeDetail({
      id: 7,
      ts: row.ts,
      endpoint: "chat",
      purpose: row.purpose,
      round: 1,
      trigger: row.trigger,
      decision_reason: row.decision_reason,
      prompt_template: "decide_v1",
      request_json: null,
      response_json: null,
      prompt_tokens: 10,
      completion_tokens: 2,
      validator_verdict: "pass",
      error: null,
      instrument_name: "Infosys",
      instrument_symbol: "INFY.NS",
    });
    expect(detail).toContain("Why it was asked");
    expect(detail).toContain("Refused");
    expect(detail).toContain("Prompt type");
    expect(detail).toContain("Infosys");
    expect(detail).toContain("insufficient_cash");
  });
});
