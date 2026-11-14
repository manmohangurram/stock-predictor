import { describe, expect, test } from "vitest";
import type { CallRow, Dashboard, PositionRow, ScheduledPrompt, Settings, SignalRow } from "./api";
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

describe("history dates", () => {
  test("includes the year once the fill is not from the current one", async () => {
    const { histDate } = await import("./format");
    const now = new Date("2026-10-27T12:00:00Z");
    expect(histDate("2026-08-20T10:00:00Z", now)).not.toContain("2026");
    expect(histDate("2025-08-20T10:00:00Z", now)).toContain("2025");
    expect(histDate(null, now)).toBe("—");
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

describe("the ledger is a view now", () => {
  test("the status filter keeps its icons — on a phone the word is hidden, the icon is not", async () => {
    const { ITEMS } = await import("./components/decision-filter");
    expect(ITEMS.map((i) => i.id)).toEqual(["pending", "approved", "rejected", "all"]);
    for (const item of ITEMS) expect(item.icon.length).toBeGreaterThan(10);
  });
});
