import { describe, expect, test } from "vitest";
import type { CallRow, Dashboard, PositionRow, ScheduledPrompt, Settings, SignalRow } from "./api";
import type { Limits } from "./api";
import { renderMarkdown } from "./markdown";
import { searchQuery, searchResults } from "./components/call-viewer";
import { renderExchangeDetail } from "./components/call-detail";

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





describe("history dates", () => {
  test("includes the year once the fill is not from the current one", async () => {
    const { histDate } = await import("./format");
    const now = new Date("2026-10-27T12:00:00Z");
    expect(histDate("2026-08-20T10:00:00Z", now)).not.toContain("2026");
    expect(histDate("2025-08-20T10:00:00Z", now)).toContain("2025");
    expect(histDate(null, now)).toBe("—");
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

describe("the ledger is a view now", () => {
  test("the status filter keeps its icons — on a phone the word is hidden, the icon is not", async () => {
    const { ITEMS } = await import("./components/decision-filter");
    expect(ITEMS.map((i) => i.id)).toEqual(["pending", "approved", "rejected", "all"]);
    for (const item of ITEMS) expect(item.icon.length).toBeGreaterThan(10);
  });
});
