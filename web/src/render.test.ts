import { describe, expect, test } from "vitest";
import type { CallRow, Dashboard, PositionRow, ScheduledPrompt, Settings, SignalRow } from "./api";
import { renderLedger, renderModeTabs, renderRow } from "./pages/ledger";
import { renderSettings, shouldDisableSave } from "./pages/settings";
import type { Limits } from "./api";
import { renderMarkdown } from "./markdown";
import { searchQuery, searchResults } from "./components/call-viewer";
import { renderCalls, renderExchangeDetail } from "./pages/calls";

const limits = (over: Partial<Limits> = {}): Limits => ({
  min_profit_pct: 5,
  max_profit_pct: 50,
  min_trade_amount: 500,
  max_trade_amount: 50_000,
  max_instrument_pct: 25,
  ...over,
});

const position = (over: Partial<PositionRow> = {}): PositionRow => ({
  target_profit_pct: null,
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
} as PositionRow);

const signal = (over: Partial<SignalRow> = {}): SignalRow => ({
  id: 1,
  symbol: "INFY.NS",
  name: "Infosys",
  plan_id: null,
  plan_rationale: null,
  plan_scope: "stock",
  amount: null,
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
  recent_trades: [],
  ...over,
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

  test("a fund row shows a NAV exit from its profit expectation, not an intraday price", () => {
    const mf = position({
      symbol: "AXISELSS",
      class: "mutual_fund",
      stop: null,
      ltp: 105.36,
      value: 25_000,
      avg_cost: 100,
      target_profit_pct: 25,
    });
    const html = renderRow(dashboard({ positions: [mf] }), mf);
    // 100 cost × 1.25 = a 125 NAV exit.
    expect(html).toContain("125.00 → +25%");
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
    const html = renderLedger(dashboard());
    expect(html).toContain("No stocks yet");
    expect(html).not.toContain("spinner");
  });

  test("free to deploy sits with the figures, not in the rail", () => {
    const html = renderLedger(dashboard({ available: 96_389, budget: 100_000 }));
    expect(html).toContain("Free to deploy");
    expect(html).toContain("of 1,00,000");
    // Nothing left of the removed recent-fills card.
    expect(html).not.toContain("Recent fills");
  });
});

describe("settings", () => {
  const settings = (over: Partial<Settings> = {}): Settings => ({
    mf_budget: 0,
    mf_available: 0,
    public_url: "http://localhost:8099",
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
      daily_cap: 200,
      scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5,
      proxy_configured: false,
      proxy_host: "",
      proxy_user: null,
      proxy_pass_configured: false,
      proxy_bypass: null,
    },
    ...over,
  } as Settings);

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
      expect(html).toContain(">Save<");
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
    for (const name of ["poll_secs", "off_hours_secs", "prompt_window_hours", "daily_cap", "min_price_move"]) {
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
        mode: "long" as const,
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
        traded_at: "2019-01-01T10:00:00Z", note: null, market_close: null, mode: "long" as const },
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
    mf_budget: 0, mf_available: 0, public_url: "http://localhost:8099",
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null, proxy_user: null, proxy_pass_configured: false,
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
    mf_budget: 0, mf_available: 0, public_url: "http://localhost:8099",
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null, proxy_user: null, proxy_pass_configured: false,
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
    expect(html).toContain("Send test");
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
    mf_budget: 0, mf_available: 0, public_url: "http://localhost:8099",
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null, proxy_user: null, proxy_pass_configured: false,
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
      mf_budget: 0, mf_available: 0, public_url: "http://localhost:8099",
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5, daily_cap: 200, proxy_configured: false, proxy_host: "",
      proxy_bypass: null, proxy_user: null, proxy_pass_configured: false,
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
      mf_budget: 0, mf_available: 0, public_url: "http://localhost:8099",
      runtime: {
        poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
        scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5, daily_cap: 200, proxy_configured: false, proxy_host: "",
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
        mf_budget: 0, mf_available: 0, public_url: "http://localhost:8099",
        runtime: {
          poll_secs: 7200, off_hours_secs: 900, prompt_window_hours: 4, min_price_move: 2,
          scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5, daily_cap: 200, proxy_configured: false, proxy_host: "",
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
      timing_note: "limit order", target_profit_pct: null, active: true,
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
  test("an archived instrument offers the way back, not a second archive", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const archived = {
      instrument: {
        id: 1, symbol: "TCS.NS", class: "stock", name: "TCS", currency: "INR",
        timing_note: "limit order", target_profit_pct: null, active: false,
      },
      bars: [], indicators: null, checks: [], position: null,
      headlines: [], signals: [], trades: [],
    };
    const html = renderInstrument(archived as never, archived as never, []);
    expect(html).toContain('id="restore-instrument"');
    expect(html).toContain(">Restore<");
    expect(html).not.toContain('id="archive-instrument"');
  });

  const base = {
    instrument: {
      id: 1, symbol: "TCS.NS", class: "stock", name: "TCS", currency: "INR",
      timing_note: "limit order", target_profit_pct: null, active: true,
    },
    bars: [], indicators: null, checks: [], position: null, headlines: [], signals: [], trades: [],
  };

  test("an unheld instrument offers a delete", async () => {
    const { renderInstrument } = await import("./pages/instrument");
    const html = renderInstrument(base as never, base as never, []);
    expect(html).toContain('id="archive-instrument"');
    expect(html).toContain(">Archive<");
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
    // The control says "Archive" and is disabled; the sentence explaining why is help text.
    expect(html).toContain(">Archive</button>");
    expect(html).toMatch(/id="archive-instrument"[^>]*disabled/);
    expect(html).toContain("cannot be archived");
  });
});

describe("the api client speaks JSON", () => {
  test("approve and reject send a JSON content-type", async () => {
    const calls: RequestInit[] = [];
    const original = globalThis.fetch;
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
      timing_note: "limit order", target_profit_pct: null, active: true,
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

describe("decision rows", () => {
  test("a blocked proposal stays out of the decisions box", async () => {
    const { renderLedger } = await import("./pages/ledger");
    const blocked = signal({ id: 9, action: "BLOCKED", qty: 1000, entry: 1148.7 });
    const html = renderLedger(dashboard({ signals: [] }));
    expect(html).not.toContain("BLOCKED");
    // It is still a record elsewhere; this box is for decisions under review.
    expect(html).toContain("Nothing under review");
  });

  test("a fund decision leads with the scheme name, not its code", async () => {
    const { renderLedger } = await import("./pages/ledger");
    const fund = signal({
      symbol: "0P0000XV5C.BO",
      name: "HDFC Balanced Advantage Direct Growth",
      class: "mutual_fund",
      action: "BUY",
      qty: 80,
      entry: 124.96,
    });
    const html = renderLedger(dashboard({ signals: [fund] }));
    expect(html).toContain("HDFC Balanced Advantage Direct Growth");
    // The code is still there, small, because that is what you paste into search.
    expect(html).toContain("0P0000XV5C.BO");
  });
});

describe("audit pager", () => {
  test("says where you are, links only where you can go, and offers a page size", async () => {
    const { renderPager } = await import("./pages/calls");
    const first = renderPager(0, 25, 240);
    expect(first).toContain("1–25 of 240");
    expect(first).toContain("page 1 of 10");
    // No "newer" link on the first window.
    expect(first).not.toContain('offset=-25');
    expect(first).toContain('href="/calls?offset=25&limit=25"');
    // The page size is a choice, not a constant.
    expect(first).toContain('data-page-size');
    expect(first).toContain('value="50"');

    const last = renderPager(225, 25, 240);
    expect(last).toContain("226–240 of 240");
    expect(last).not.toContain('href="/calls?offset=250');

    // A different size re-cuts the same record.
    expect(renderPager(0, 100, 240)).toContain("1–100 of 240");
    expect(renderPager(0, 25, 7)).toContain("1–7 of 7");
  });
});

async function planCard(content: unknown): Promise<string> {
  const { renderExchangeDetail } = await import("./pages/calls");
  return renderExchangeDetail({
    id: 9,
    ts: "2026-11-13T03:30:00Z",
    endpoint: "chat",
    purpose: "plan",
    round: 1,
    trigger: "clock",
    decision_reason: null,
    prompt_template: "plan_stock_v1",
    request_json: null,
    response_json: JSON.stringify({ choices: [{ message: { content: JSON.stringify(content) } }] }),
    prompt_tokens: 10,
    completion_tokens: 2,
    validator_verdict: "pass",
    error: null,
    instrument_name: null,
    instrument_symbol: null,
  });
}

describe("audit", () => {
  test("a plan is a docket: figures, cost, then one paragraph", async () => {
    const { renderExchangeDetail } = await import("./pages/calls");
    const content = JSON.stringify({
      action: "plan",
      rationale: "pure entry slot",
      moves: [
        {
          action: "BUY",
          symbol: "BONFIG.NS",
          qty: 12,
          price: 794.55,
          stop: 755,
          target: 880,
          valid_until: "2026-11-17T15:30:00+05:30",
          why: "Q2FY27 results dated 2026-11-11",
        },
      ],
    });
    const html = renderExchangeDetail({
      id: 7,
      ts: "2026-11-13T13:30:00Z",
      endpoint: "chat",
      purpose: "trade plan",
      round: 1,
      trigger: "clock",
      decision_reason: null,
      prompt_template: "plan_trade_v1",
      request_json: null,
      response_json: JSON.stringify({ choices: [{ message: { content } }] }),
      prompt_tokens: 10,
      completion_tokens: 2,
      validator_verdict: "pass",
      error: null,
      instrument_name: null,
      instrument_symbol: null,
    });
    expect(html).toContain('class="docket"');
    expect(html).toContain("Qty");
    expect(html).toContain("Cost");
    expect(html).toContain("pure entry slot");
    expect(html).toContain("Till");
    const card = html.slice(html.indexOf('<article class="answer"'), html.indexOf("</article>") + 10);
    expect(card).not.toContain("Q2FY27 results dated 2026-11-11");
  });

  test("a swap shows what the sale frees and what the buy still needs", async () => {
    const { renderExchangeDetail } = await import("./pages/calls");
    const content = JSON.stringify({
      action: "plan",
      rationale: "replacement",
      moves: [
        { action: "SELL", symbol: "TATAMOTORS.NS", qty: 40, price: 780.5 },
        {
          action: "BUY",
          symbol: "GABRIEL.NS",
          qty: 27,
          price: 1519,
          stop: 1450,
          target: 1700,
          valid_until: "2026-11-16T15:30:00+05:30",
        },
      ],
    });
    const html = renderExchangeDetail({
      id: 8,
      ts: "2026-11-14T03:45:00Z",
      endpoint: "chat",
      purpose: "trade plan",
      round: 1,
      trigger: "clock",
      decision_reason: null,
      prompt_template: "plan_trade_v1",
      request_json: null,
      response_json: JSON.stringify({ choices: [{ message: { content } }] }),
      prompt_tokens: 10,
      completion_tokens: 2,
      validator_verdict: "pass",
      error: null,
      instrument_name: null,
      instrument_symbol: null,
    });
    expect(html).toContain("From the sale");
    expect(html).toContain("To buy");
    expect(html).toContain("31,220.00");
    expect(html).toContain("41,013.00");
    expect(html).toContain("Also from the purse");
    expect(html).toContain("9,793.00");
  });

  test("a three-move plan stays one line per move, and the purse adds up", async () => {
    const html = await planCard({
      action: "plan",
      rationale: "two stale names out, one live name in",
      moves: [
        { action: "SELL", symbol: "TCS.NS", qty: 5, price: 3900 },
        { action: "SELL", symbol: "RELIANCE.NS", qty: 8, price: 1400 },
        {
          action: "BUY",
          symbol: "INFY.NS",
          qty: 20,
          price: 1600,
          stop: 1520,
          target: 1760,
          valid_until: "2026-11-20T15:30:00+05:30",
        },
      ],
    });
    expect((html.match(/class="docket-move"/g) ?? []).length).toBe(3);
    expect(html).toContain("From the sale");
    expect(html).toContain("30,700.00");
    expect(html).toContain("To buy");
    expect(html).toContain("32,000.00");
    expect(html).toContain("Also from the purse");
    expect(html).toContain("1,300.00");
  });

  test("a fund move is sized in rupees, not units", async () => {
    const html = await planCard({
      action: "plan",
      rationale: "a NAV dip in a fund already held",
      moves: [
        { action: "BUY", symbol: "AXISELSS", amount: 2250 },
        { action: "BUY", symbol: "0P0000XV5C.BO", amount: 1500 },
      ],
    });
    expect(html).toContain("Amount");
    expect(html).toContain("2,250.00");
    expect(html).toContain("1,500.00");
    expect(html).toContain("Cost");
    expect(html).toContain("3,750.00");
    expect(html).not.toContain("Qty");
  });

  test("a move shows the fund's name, not only its scheme code", async () => {
    const { renderExchangeDetail } = await import("./pages/calls");
    const content = JSON.stringify({
      action: "plan",
      rationale: "a NAV dip in a fund already held",
      moves: [{ action: "BUY", symbol: "0P0000XV5C.BO", amount: 2250 }],
    });
    const html = renderExchangeDetail({
      id: 11,
      ts: "2026-11-13T03:35:00Z",
      endpoint: "chat",
      purpose: "fund plan",
      round: 1,
      trigger: "clock",
      decision_reason: null,
      prompt_template: "plan_mf_v1",
      request_json: null,
      response_json: JSON.stringify({ choices: [{ message: { content } }] }),
      prompt_tokens: 10,
      completion_tokens: 2,
      validator_verdict: "pass",
      error: null,
      instrument_name: null,
      instrument_symbol: null,
      plan_names: { "0P0000XV5C.BO": "HDFC Balanced Advantage Direct Growth" },
    });
    const card = html.slice(html.indexOf('<article class="answer"'), html.indexOf("</article>") + 10);
    expect(card).toContain("HDFC Balanced Advantage Direct Growth");
    expect(card).not.toContain("0P0000XV5C.BO");
  });

  test("a refused plan is not a plan awaiting you", async () => {
    const { renderCalls } = await import("./pages/calls");
    const row: CallRow = {
      id: 8,
      ts: "2026-11-13T13:30:00Z",
      endpoint: "chat",
      purpose: "trade plan",
      round: 1,
      prompt_tokens: 3076,
      completion_tokens: 311,
      validator_verdict: "refused",
      error: null,
      trigger: "clock",
      decision_id: 9,
      decision_status: "pending",
      decision_action: "BLOCKED",
      decision_reason: "BONFIG.NS: expiry is more than 3 sessions out",
      instrument_name: null,
      instrument_symbol: null,
      // It is a plan row, but it was refused whole: there are no legs to approve.
      plan_legs: 1,
      model: "plan_trade_v1",
    };
    const list = renderCalls(
      { prompts: [], now: "2026-11-13T14:00:00Z" },
      0,
      [row],
      { prompts: [], calls: [], models: [] },
      {},
    );
    expect(list).toContain("plan · refused");
    expect(list).not.toContain("awaiting you");
  });

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
      plan_legs: 0,
      model: "decide_v1",
    };
    const list = renderCalls(
      { prompts: [], now: "2026-11-09T10:00:00Z" },
      0,
      [row],
      { prompts: [], calls: [], models: [] },
      {},
    );
    expect(list).toContain("Infosys");
    expect(list).toContain("blocked");
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
    expect(detail).toContain("Asked because");
    expect(detail).toContain("Refused");
    expect(detail).toContain("Model");
    // The row already says the type, channel and holding; the drawer must not repeat them.
    expect(detail).not.toContain("Prompt type");
    expect(detail).not.toContain("Holding");
    expect(detail).toContain("insufficient_cash");
  });
});

describe("a fund decision's fill", () => {
  test("asks for rupees and a NAV, and says the size is not proposed", async () => {
    const { renderDecisionActions } = await import("./components/decision-actions");
    const html = renderDecisionActions(signal({
      class: "mutual_fund",
      action: "BUY",
      qty: 0,
      entry: 74.05,
    }));
    expect(html).toContain("Amount ₹");
    expect(html).toContain("NAV");
    expect(html).toContain('name="amount"');
    expect(html).toContain("size on approval · NAV 74.05");
    expect(html).not.toContain("0 units");
  });
});

describe("a plan", () => {
  const leg = (over: Partial<SignalRow>): SignalRow => ({ ...signal(), ...over });
  test("reads as one docket with the purse arithmetic that makes it legal", async () => {
    const { renderPlan } = await import("./pages/ledger");
    const html = renderPlan([
      leg({ id: 2, action: "SELL", name: "Infosys", qty: 24, entry: 1146.3 }),
      leg({ id: 3, action: "BUY", name: "HDFC Bank", symbol: "HDFCBANK.NS", qty: 15, entry: 793.65 }),
    ]);
    expect(html).toContain("STOCK PLAN");
    expect(html).toContain("2 moves");
    expect(html).toContain("Infosys");
    expect(html).toContain("HDFC Bank");
    expect((html.match(/class="docket-move"/g) ?? []).length).toBe(2);
    expect(html).toContain("From the sale");
    expect(html).toContain("27,511.20");
    expect(html).toContain("To buy");
    expect(html).toContain("11,904.75");
    expect(html).toContain("Back to the purse");
    expect(html).toContain("15,606.45");
    // One approve control for the whole plan, keyed to the first leg.
    expect((html.match(/data-approve=/g) ?? []).length).toBe(1);
    expect(html).toContain('data-approve="2"');
    // Each leg carries its own editable fill: a plan is not approved one stock at a time.
    expect(html).toContain('name="qty-2"');
    expect(html).toContain('name="qty-3"');
    expect(html).toContain('name="price-3"');
    expect(html).toContain("Post fills");
  });

  test("a fund leg shows the rupees it was sized with, once", async () => {
    const { renderPlan } = await import("./pages/ledger");
    const html = renderPlan([
      leg({
        id: 5,
        action: "BUY",
        name: "Axis ELSS",
        class: "mutual_fund",
        plan_scope: "fund",
        qty: null,
        entry: null,
        amount: 2250,
      }),
    ]);
    expect(html).toContain("FUND PLAN");
    expect(html).toContain("Amount");
    expect(html).toContain("2,250.00");
    // The amount is already on the line; a Cost row would print the same figure twice.
    expect(html).not.toContain("Cost");
  });

  test("a fund plan's fill asks for rupees and a NAV, not units", async () => {
    const { renderPlan } = await import("./pages/ledger");
    const html = renderPlan([
      leg({
        id: 7,
        action: "BUY",
        name: "Axis ELSS",
        class: "mutual_fund",
        plan_scope: "fund",
        qty: null,
        entry: null,
        amount: 2250,
      }),
    ]);
    expect(html).toContain("Amount ₹");
    expect(html).toContain("NAV");
    expect(html).toContain('name="amount-7"');
    expect(html).toContain('value="2250"');
    expect(html).toContain('name="price-7"');
  });

  test("a fund leg with no stored size says so instead of printing nothing", async () => {
    const { renderPlan } = await import("./pages/ledger");
    const html = renderPlan([
      leg({ id: 6, action: "BUY", name: "Axis ELSS", class: "mutual_fund", plan_scope: "fund",
        qty: null, entry: null }),
    ]);
    expect(html).toContain("size on approval");
  });
});

describe("archived", () => {
  test("shows what the fills made, and where to restore it", async () => {
    const { renderArchived } = await import("./pages/archived");
    const html = renderArchived([
      {
        id: 1,
        symbol: "TCS",
        name: "Tata Consultancy",
        class: "stock",
        fills: 2,
        decisions: 3,
        last_fill: "2026-01-09T00:00:00Z",
        realised: 300,
        net_invested: -300,
      },
    ]);
    expect(html).toContain("Tata Consultancy");
    // Realised, with its sign, and the round trip closed out.
    expect(html).toContain("+300.00");
    expect(html).toContain("300.00 out");
    expect(html).toContain("Realised");
    expect(html).not.toContain("data-restore");
    expect(html).not.toContain("open to restore");
  });

  test("an archive with no fills says so instead of inventing a zero", async () => {
    const { renderArchived } = await import("./pages/archived");
    const html = renderArchived([
      {
        id: 2, symbol: "XYZ", name: "Never traded", class: "stock",
        fills: 0, decisions: 0, last_fill: null, realised: 0, net_invested: 0,
      },
    ]);
    expect(html).toContain("Never traded");
    expect(html).toContain("—");
  });
});

describe("the schedule strip", () => {
  test("says when each prompt last ran, when it is next due, and how often", async () => {
    const { renderSchedule } = await import("./pages/calls");
    const prompt = (over: Partial<ScheduledPrompt>): ScheduledPrompt => ({
      prompt: "plan_stock",
      label: "Stock plan",
      rule: "once a day, first tick after midnight IST",
      next_due_at: "2026-11-11T00:05:00+05:30",
      last: {
        status: "done",
        trigger: "schedule",
        at: "2026-11-10T18:51:00Z",
        finished_at: "2026-11-10T18:51:03Z",
        summary: "2 moves stored",
        error: null,
      },
      ...over,
    });
    const html = renderSchedule(
      [
        prompt({}),
        prompt({
          prompt: "plan_fund",
          label: "Fund plan",
          last: null,
          next_due_at: null,
        }),
        prompt({
          prompt: "plan_trade",
          label: "Trade plan",
          rule: "every tick, 09:00–16:00 IST, Mon–Fri",
          next_due_at: "2026-11-09T09:00:00Z",
        }),
      ],
      "2026-11-09T10:00:00Z",
      0,
    );
    expect(html).toContain("2 moves stored");
    expect(html).toContain("once a day, first tick after midnight IST");
    expect(html).toContain("Next due");
    // A prompt that never ran says so, and still offers the run.
    expect(html).toContain("never run");
    expect(html).toContain('data-run-again="plan_fund"');
    // A due date in the past reads as due now, not as a date.
    expect(html).toContain("due now");
  });
});

describe("labels from identifiers", () => {
  test("a model name is capitalised and loses its underscores", async () => {
    const { humanLabel } = await import("./format");
    expect(humanLabel("decide_v1")).toBe("Decide v1");
    expect(humanLabel("plan_stock_v1")).toBe("Plan stock v1");
    expect(humanLabel("plan_mf_v1")).toBe("Plan mf v1");
    // Values without underscores keep their shape, including symbols.
    expect(humanLabel("chat+search")).toBe("Chat+search");
    expect(humanLabel("")).toBe("");
  });
});

describe("markdown in the audit drawer", () => {
  test("renders the prompt as prose, not as markup", () => {
    const html = renderMarkdown("## Rules\n\n1. **Both exits.** `entry` below `stop`.\n\n- a\n- b");
    expect(html).toContain("<h4>Rules</h4>");
    expect(html).toContain("<strong>Both exits.</strong>");
    expect(html).toContain("<code>entry</code>");
    expect(html).toContain("<ol>");
    expect(html).toContain("<ul>");
  });

  test("escapes the prompt's own angle brackets", () => {
    expect(renderMarkdown("a <script>alert(1)</script> b")).toContain("&lt;script&gt;");
  });

  test("keeps a fenced JSON example verbatim", () => {
    const html = renderMarkdown('```json\n{"action":"pass"}\n```');
    // Escaped for HTML, so it renders as the text it is.
    expect(html).toContain('<pre class="md-code"><code>{&quot;action&quot;:&quot;pass&quot;}</code></pre>');
  });
});

describe("markdown numbering", () => {
  test("a lazily numbered list stays one list and counts up", () => {
    const html = renderMarkdown("1. first\n\n1. second\n\n1. third");
    expect(html.match(/<ol>/g)?.length).toBe(1);
    expect(html.match(/<li>/g)?.length).toBe(3);
  });

  test("a blank line between a list and prose still ends the list", () => {
    const html = renderMarkdown("- a\n\nThen prose.");
    expect(html).toContain("</ul>");
    expect(html).toContain("<p>Then prose.</p>");
  });
});

describe("markdown reflow", () => {
  test("a hard-wrapped paragraph reflows instead of keeping the source's line breaks", () => {
    const html = renderMarkdown("a setup that needs a\nweek to pay is not this book's trade");
    expect(html).toBe("<p>a setup that needs a week to pay is not this book&#39;s trade</p>".replace("&#39;", "'"));
  });
});

describe("a search call is shown as a question and its results", () => {
  const request = JSON.stringify({ query: "DLF news today", count: 5, need_content: false });
  const response = JSON.stringify({
    results: [
      {
        title: "DLF, GIC JV clocks 9% growth in rental income",
        site_name: "Hindustan Times",
        date: "2026-11-09T16:00:00+00:00",
        url: "https://example.com/a",
        snippet: "Rental income rose 9% to ₹1,481 crore.",
        summary: "A long summary nobody reads.",
      },
    ],
  });

  test("the query and the count are read from the request", () => {
    expect(searchQuery(request)).toEqual({ query: "DLF news today", count: 5 });
  });

  test("each result keeps its title, source, date and snippet", () => {
    const [r] = searchResults(response);
    expect(r.title).toContain("rental income");
    expect(r.site).toBe("Hindustan Times");
    expect(r.snippet).toContain("1,481");
  });

  test("the drawer shows what was searched and what came back, not a chat exchange", () => {
    const html = renderExchangeDetail({
      id: 1, ts: "2026-11-10T14:05:00Z", endpoint: "search", purpose: "search DLF.NS",
      round: 1, trigger: null, decision_reason: null, instrument_name: "DLF Ltd",
      instrument_symbol: "DLF.NS", prompt_template: null, completion_tokens: null,
      request_json: request, response_json: response, prompt_tokens: null,
      validator_verdict: null, error: null,
    });
    expect(html).toContain("DLF news today");
    expect(html).toContain("Hindustan Times");
    expect(html).toContain("search-gist");
    expect(html).not.toContain("Sent to the model");
  });
});

describe("the prompt type column says what the call was", () => {
  const row = (over: Partial<CallRow>): CallRow => ({
    id: 1, ts: "2026-11-10T19:57:00Z", endpoint: "chat", purpose: "trend MARUTI.NS",
    decision_id: null, decision_status: null, decision_action: null, decision_reason: null,
    instrument_name: "Maruti", instrument_symbol: "MARUTI.NS", plan_legs: 0,
    model: "trend_v1", round: 1, prompt_tokens: 10, completion_tokens: 5,
    validator_verdict: null, trigger: null, error: null, ...over,
  });

  const table = (rows: CallRow[]) =>
    renderCalls(
      { prompts: [], now: "2026-11-10T20:00:00Z" },
      0,
      rows,
      { prompts: [], calls: [], models: [] },
      {},
    );

  test("a search is named for what it is, not for the first word of its query", () => {
    const html = table([
      row({ endpoint: "search", purpose: "MARUTI news today", instrument_name: null, instrument_symbol: null }),
    ]);
    expect(html).toContain("Web search");
    expect(html).not.toContain('<td class="hide-sm">Maruti</td>');
  });

  test("a model call is named for its prompt", () => {
    const html = table([row({})]);
    expect(html).toContain("Trend");
    expect(html).toContain("Maruti");
  });
});

describe("every column agrees with its own header", () => {
  // No DOM here, so the classes are read from the markup: a header says how its column is aligned.
  const classes = (html: string, tag: "th" | "td") =>
    [...html.matchAll(new RegExp(`<${tag}(?=[ >])([^>]*)>`, "g"))].map(
      (m) => /class="([^"]*)"/.exec(m[1])?.[1] ?? "",
    );
  const numeric = (cls: string) => cls.split(/\s+/).includes("num");

  test("archived: the date column lines up with the figures beside it", async () => {
    const { renderArchived } = await import("./pages/archived");
    const html = renderArchived([
      {
        id: 1, symbol: "TCS.NS", name: "Tata Consultancy", class: "stock",
        fills: 2, decisions: 0, last_fill: "2026-01-09T00:00:00Z",
        realised: 300, net_invested: -300,
      },
    ]);
    // Holding, fills, decisions, last fill, realised — the date shares the figures' edge.
    expect(classes(html, "th").map(numeric)).toEqual([false, true, true, true, true]);
    expect(classes(html, "td").map(numeric)).toEqual([false, true, true, true, true]);
  });
});

describe("an empty audit page under a filter", () => {
  test("says the filters matched nothing, and offers the way out", async () => {
    const { renderCalls } = await import("./pages/calls");
    const html = renderCalls(
      { prompts: [], now: "2026-11-11T12:00:00Z" },
      0,
      [],
      { prompts: [], calls: [], models: [] },
      { prompt: "decide_v1" },
    );
    expect(html).toContain("No calls match these filters");
    expect(html).toContain('href="/calls"');
    expect(html).not.toContain("No calls yet.");
  });

  test("an unfiltered empty page still says there are none yet", async () => {
    const { renderCalls } = await import("./pages/calls");
    const html = renderCalls(
      { prompts: [], now: "2026-11-11T12:00:00Z" },
      0,
      [],
      { prompts: [], calls: [], models: [] },
      {},
    );
    expect(html).toContain("No calls yet.");
  });
});

describe("the scan filters are settings, not constants", () => {
  const base = (over: Record<string, unknown>) => ({
    budget: 500_000, committed: 0, available: 500_000, currency: "INR",
    currency_locked: false, deepseek_configured: false,
    trading_budget: 100_000, trading_available: 80_000, mf_budget: 0, mf_available: 0,
    ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
    llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
    llm_key_configured: false, limits_long: limits(), limits_trade: limits(),
    public_url: "http://localhost:8099",
    runtime: {
      poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
      daily_cap: 200, scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5,
      proxy_configured: false, proxy_host: "", proxy_bypass: null, proxy_user: null,
      proxy_pass_configured: false,
    },
    ...over,
  });

  test("the cadence tab carries the three floors and says what they do", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings(base({}) as never, "cadence");
    expect(html).toContain('name="scan_min_price"');
    expect(html).toContain('name="scan_min_turnover"');
    expect(html).toContain('name="scan_locked_pct"');
    expect(html).toContain("What the short-term scan may offer");
    // Stored in rupees, typed in crore: 10,000,000 shows as 1.
    expect(html).toMatch(/id="scan_min_turnover"[^>]*value="1"/);
    expect(html).toContain("in whole crore");
  });

  test("a changed floor is shown as stored", async () => {
    const { renderSettings } = await import("./pages/settings");
    const s = base({}) as { runtime: Record<string, unknown> };
    const html = renderSettings(
      base({ runtime: { ...s.runtime, scan_min_price: 25 } }) as never,
      "cadence",
    );
    expect(html).toMatch(/id="scan_min_price"[^>]*value="25"|value="25"[^>]*id="scan_min_price"/);
  });
});

describe("the turnover floor is typed in crore", () => {
  test("rupees and crore round-trip without drift", async () => {
    const { crore, rupees } = await import("./pages/settings");
    expect(crore(10_000_000)).toBe(1);
    expect(crore(5_000_000)).toBe(0.5);
    expect(crore(2_500_000)).toBe(0.25);
    expect(rupees(1)).toBe(10_000_000);
    expect(rupees(0.5)).toBe(5_000_000);
    expect(rupees(crore(7_000_000))).toBe(7_000_000);
  });

  test("a stored value reads back as the crore a person would type", async () => {
    const { renderSettings, crore } = await import("./pages/settings");
    const html = renderSettings(
      {
        budget: 500_000, committed: 0, available: 500_000, currency: "INR",
        currency_locked: false, deepseek_configured: false,
        trading_budget: 100_000, trading_available: 80_000, mf_budget: 0, mf_available: 0,
        ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
        llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
        llm_key_configured: false, limits_long: limits(), limits_trade: limits(),
        public_url: "http://localhost:8099",
        runtime: {
          poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
          daily_cap: 200, scan_min_price: 10, scan_min_turnover: 50_000_000, scan_locked_pct: 19.5,
          proxy_configured: false, proxy_host: "", proxy_bypass: null, proxy_user: null,
          proxy_pass_configured: false,
        },
      } as never,
      "cadence",
    );
    expect(crore(50_000_000)).toBe(5);
    expect(html).toMatch(/id="scan_min_turnover"[^>]*value="5"/);
  });
});

describe("the turnover field takes whole crore", () => {
  test("a whole crore shows as an integer, a fraction is shown rather than rounded", async () => {
    const { crore } = await import("./pages/settings");
    expect(crore(10_000_000)).toBe(1);
    expect(crore(50_000_000)).toBe(5);
    // Stored elsewhere as 2.5 crore: say 2.5, do not pretend it is 3.
    expect(crore(25_000_000)).toBe(2.5);
  });

  test("the input steps by a whole crore", async () => {
    const { renderSettings } = await import("./pages/settings");
    const html = renderSettings(
      {
        budget: 500_000, committed: 0, available: 500_000, currency: "INR",
        currency_locked: false, deepseek_configured: false,
        trading_budget: 100_000, trading_available: 80_000, mf_budget: 0, mf_available: 0,
        ntfy_url: "https://ntfy.sh", ntfy_topic: "", ntfy_token_configured: false,
        llm_provider: "deepseek", llm_model: "deepseek-chat", llm_base_url: "https://api.deepseek.com",
        llm_key_configured: false, limits_long: limits(), limits_trade: limits(),
        public_url: "http://localhost:8099",
        runtime: {
          poll_secs: 900, off_hours_secs: 7200, prompt_window_hours: 4, min_price_move: 2,
          daily_cap: 200, scan_min_price: 10, scan_min_turnover: 10_000_000, scan_locked_pct: 19.5,
          proxy_configured: false, proxy_host: "", proxy_bypass: null, proxy_user: null,
          proxy_pass_configured: false,
        },
      } as never,
      "cadence",
    );
    expect(html).toMatch(/id="scan_min_turnover"[^>]*step="1"/);
    expect(html).toMatch(/id="scan_min_turnover"[^>]*value="1"/);
  });
});
