export interface SignalRow {
  id: number;
  symbol: string;
  class: "stock" | "mutual_fund" | "ipo";
  action: string;
  qty: number | null;
  confidence: string | null;
  entry: number | null;
  stop: number | null;
  target: number | null;
  valid_until: string | null;
  timing_note: string;
  reasons: string[];
  rationale: string | null;
  created_at: string;
}

export interface PositionRow {
  id: number;
  symbol: string;
  class: "stock" | "mutual_fund" | "ipo";
  qty: number;
  avg_cost: number;
  ltp: number | null;
  value: number | null;
  stop: number | null;
  target: number | null;
  state: string;
  timing_note: string;
}

export type Mode = "long" | "trade";

export interface Dashboard {
  currency: string;
  mode: Mode;
  budget: number;
  committed: number;
  available: number;
  realised: number;
  currency_locked: boolean;
  positions: PositionRow[];
  signals: SignalRow[];
  blocked: SignalRow[];
  recent_trades: Trade[];
}

export interface Trade {
  id: number;
  symbol: string;
  side: "buy" | "sell";
  mode: Mode;
  qty: number;
  price: number;
  realised: number | null;
  traded_at: string;
  note: string | null;
  market_close: number | null;
}

export interface Hit {
  code: string;
  name: string;
  class: "stock" | "mutual_fund" | "ipo";
  venue: string;
  price: number | null;
}

export interface Settings {
  budget: number;
  committed: number;
  available: number;
  currency: string;
  currency_locked: boolean;
  deepseek_configured: boolean;
  trading_budget: number;
  trading_available: number;
  ntfy_url: string;
  ntfy_topic: string;
  ntfy_token_configured: boolean;
  public_url: string;
  llm_provider: string;
  llm_model: string;
  llm_base_url: string;
  llm_key_configured: boolean;
  limits_long: Limits;
  limits_trade: Limits;
}

export interface Limits {
  min_profit_pct: number;
  max_profit_pct: number;
  min_trade_amount: number;
  max_trade_amount: number;
  max_instrument_pct: number;
}

export interface CallRow {
  id: number;
  ts: string;
  endpoint: string;
  purpose: string | null;
  round: number | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  validator_verdict: string | null;
  error: string | null;
}

export interface BarPoint {
  ts: string;
  close: number;
}

export interface CheckRow {
  name: string;
  passed: boolean;
  detail: string;
}

export interface Analysis {
  instrument: {
    id: number;
    symbol: string;
    class: string;
    name: string;
    currency: string;
    timing_note: string;
    target_profit_pct: number | null;
  };
  bars: BarPoint[];
  indicators: {
    sma50: number;
    sma200: number;
    rsi14: number;
    atr14: number;
    high52: number;
    low52: number;
  } | null;
  checks: CheckRow[];
  position: PositionRow | null;
  headlines: { id: number; title: string; publisher: string | null; published_at: string; url: string }[];
  signals: SignalRow[];
  trades: Trade[];
}

export interface InstrumentDetail {
  instrument: {
    id: number;
    symbol: string;
    class: string;
    name: string;
    currency: string;
    timing_note: string;
    target_profit_pct: number | null;
  };
  headlines: { id: number; title: string; publisher: string | null; published_at: string; url: string }[];
  signals: SignalRow[];
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, init);
  if (!r.ok) {
    const body = await r.text();
    throw new Error(`${path}: ${r.status} ${body.slice(0, 200)}`);
  }
  return r.json() as Promise<T>;
}

const json = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify(body),
});

/**
 * Conditional dashboard fetch. The backend answers 304 when nothing changed, so a quiet poll
 * costs no body and — more visibly — no re-render, which would otherwise collapse any row the
 * user had expanded.
 */
export async function dashboardCached(
  mode: Mode,
  etag: string | null,
): Promise<{ notModified: boolean; data?: Dashboard; etag?: string }> {
  const headers: Record<string, string> = {};
  if (etag) headers["if-none-match"] = etag;
  const r = await fetch(`/api/dashboard?mode=${mode}`, { headers });
  if (r.status === 304) return { notModified: true };
  if (!r.ok) throw new Error(`dashboard: ${r.status}`);
  return {
    notModified: false,
    data: (await r.json()) as Dashboard,
    etag: r.headers.get("etag") ?? undefined,
  };
}

export const api = {
  dashboard: (mode: Mode = "long") => req<Dashboard>(`/api/dashboard?mode=${mode}`),
  settings: () => req<Settings>("/api/settings"),
  calls: () => req<CallRow[]>("/api/calls"),
  trades: () => req<Trade[]>("/api/trades"),
  instrument: (symbol: string) =>
    req<InstrumentDetail>(`/api/instruments/${encodeURIComponent(symbol)}`),
  instrumentTrades: (symbol: string) =>
    req<Trade[]>(`/api/instruments/${encodeURIComponent(symbol)}/trades`),
  analysis: (symbol: string) =>
    req<Analysis>(`/api/instruments/${encodeURIComponent(symbol)}/analysis`),
  search: (q: string) => req<Hit[]>(`/api/search?q=${encodeURIComponent(q)}`),
  setTargetProfit: (symbol: string, pct: number | null) =>
    req<{ symbol: string; target_profit_pct: number | null }>(
      `/api/instruments/${encodeURIComponent(symbol)}/target-profit`,
      { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ target_profit_pct: pct }) },
    ),
  testNotify: async () => {
    const r = await fetch("/api/notify/test", { method: "POST" });
    return { ok: r.ok, body: await r.json().catch(() => ({})) };
  },
  addInstrument: (code: string, name?: string, cls?: string) =>
    req<{ symbol: string }>("/api/instruments", json({ code, name, class: cls })),
  recordTrade: (
    symbol: string,
    side: "buy" | "sell",
    qty: number,
    price: number,
    at?: string,
    mode: Mode = "long",
  ) =>
    req<{ ok: boolean; cash: number; realised: number }>(
      `/api/instruments/${encodeURIComponent(symbol)}/trades`,
      json({ side, qty, price, at, mode }),
    ),
  putSettings: async (
    patch: Partial<
      Pick<Settings, "budget" | "currency" | "trading_budget" | "ntfy_url" | "ntfy_topic" | "public_url">
    > & { deepseek_key?: string; ntfy_token?: string },
  ) => {
    const r = await fetch("/api/settings", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    });
    return { ok: r.ok, status: r.status, body: await r.json() };
  },
};
