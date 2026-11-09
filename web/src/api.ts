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
  /** pending | approved | rejected — the ledger lists pending unless a filter says otherwise. */
  status: "pending" | "approved" | "rejected";
  status_reason: string | null;
  filled_qty: number | null;
  filled_price: number | null;
}

export interface PositionRow {
  id: number;
  symbol: string;
  /** The readable name — what the ledger shows instead of the ticker. */
  name: string;
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
  runtime: Runtime;
}

/** Cadence, gate thresholds and the proxy. The proxy itself is never returned. */
export interface Runtime {
  poll_secs: number;
  off_hours_secs: number;
  prompt_window_hours: number;
  min_price_move: number;
  max_silence_secs: number;
  daily_cap: number;
  proxy_configured: boolean;
  proxy_host: string;
  proxy_user: string | null;
  proxy_pass_configured: boolean;
  proxy_bypass: string | null;
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
  /** Why the model was asked: "price moved …", "new headline …". */
  trigger: string | null;
  decision_id: number | null;
  decision_status: "pending" | "approved" | "rejected" | null;
  purpose: string | null;
  round: number | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  validator_verdict: string | null;
  error: string | null;
}

/** One call with its stored bodies. Fetched only when a row is opened. */
export interface CallDetail {
  id: number;
  ts: string;
  endpoint: string;
  purpose: string | null;
  round: number | null;
  prompt_template: string | null;
  request_json: string | null;
  response_json: string | null;
  prompt_tokens: number | null;
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
  decisions: "pending" | "approved" | "rejected" | "all" = "pending",
): Promise<{ notModified: boolean; data?: Dashboard; etag?: string }> {
  const headers: Record<string, string> = {};
  // Only the default view is cacheable: a filtered view is a different question.
  if (etag && decisions === "pending") headers["if-none-match"] = etag;
  const q = `mode=${mode}${decisions === "pending" ? "" : `&decisions=${decisions}`}`;
  const r = await fetch(`/api/dashboard?${q}`, { headers });
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
  testProxy: () =>
    req<{ ok: boolean; status: number | null; millis: number; error: string | null }>(
      "/api/settings/proxy/test",
      { method: "POST" },
    ),
  calls: () => req<CallRow[]>("/api/calls"),
  approveSignal: (id: number, body: { qty?: number; price?: number; at?: string }) =>
    req<{ id: number; status: string; recorded: boolean }>(
      `/api/signals/${id}/approve`,
      json(body),
    ),
  rejectSignal: (id: number, reason?: string) =>
    req<{ id: number; status: string }>(`/api/signals/${id}/reject`, json({ reason })),
  refreshInstrument: (symbol: string) =>
    req<{ ok: boolean; bars: number; news: number }>(
      `/api/instruments/${encodeURIComponent(symbol)}/refresh`,
      { method: "POST" },
    ),
  deleteInstrument: (symbol: string) =>
    req<{ symbol: string; deleted: boolean; archived: boolean; reason?: string }>(
      `/api/instruments/${encodeURIComponent(symbol)}`,
      { method: "DELETE" },
    ),
  callDetail: (id: number) => req<CallDetail>(`/api/calls/${id}`),
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
      Pick<Settings, "budget" | "currency" | "trading_budget" | "ntfy_url" | "ntfy_topic" | "llm_base_url" | "llm_model" | "llm_provider">
    > & {
      deepseek_key?: string;
      ntfy_token?: string;
      llm_key?: string;
      limits_long?: Limits;
      limits_trade?: Limits;
      poll_secs?: number;
      off_hours_secs?: number;
      prompt_window_hours?: number;
      min_price_move?: number;
      max_silence_secs?: number;
      daily_cap?: number;
      proxy?: string;
      proxy_user?: string;
      proxy_pass?: string;
      proxy_bypass?: string;
    },
  ) => {
    const r = await fetch("/api/settings", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    });
    return { ok: r.ok, status: r.status, body: await r.json() };
  },
};
