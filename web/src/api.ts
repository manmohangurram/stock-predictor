export interface SignalRow {
  id: number;
  symbol: string;
  class: "stock" | "mutual_fund" | "ipo";
  action: string;
  qty: number | null;
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

export interface Dashboard {
  currency: string;
  budget: number;
  committed: number;
  available: number;
  realised: number;
  currency_locked: boolean;
  positions: PositionRow[];
  signals: SignalRow[];
  blocked: SignalRow[];
}

export interface Trade {
  id: number;
  symbol: string;
  side: "buy" | "sell";
  qty: number;
  price: number;
  realised: number | null;
  traded_at: string;
  note: string | null;
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
}

export interface InstrumentDetail {
  instrument: {
    id: number;
    symbol: string;
    class: string;
    name: string;
    currency: string;
    timing_note: string;
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

export const api = {
  dashboard: () => req<Dashboard>("/api/dashboard"),
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
  addInstrument: (code: string, name?: string, cls?: string) =>
    req<{ symbol: string }>("/api/instruments", json({ code, name, class: cls })),
  recordTrade: (symbol: string, side: "buy" | "sell", qty: number, price: number) =>
    req<{ ok: boolean; cash: number; realised: number }>(
      `/api/instruments/${encodeURIComponent(symbol)}/trades`,
      json({ side, qty, price }),
    ),
  putSettings: async (
    patch: Partial<Pick<Settings, "budget" | "currency">> & { deepseek_key?: string },
  ) => {
    const r = await fetch("/api/settings", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    });
    return { ok: r.ok, status: r.status, body: await r.json() };
  },
};
