export interface SignalRow {
  /** The uuid the server knows this decision by. */
  id: string;
  symbol: string;
  /** The readable name — a plan card says "Infosys", not "INFY.NS". */
  name: string;
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
  /** Set when this row is one leg of a plan; every leg shares the first leg's id. */
  plan_id: string | null;
  /** The sentence that explains the whole plan, carried on every leg. */
  plan_rationale: string | null;
  /** "stock" or "fund" — the two plans are different arguments. */
  plan_scope: string | null;
  /** Rupees, for a fund move sized in money rather than units. */
  amount: number | null;
}

export interface PositionRow {
  id: string;
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
  /** The owner's profit expectation. A fund's only exit: sold at NAV once the return is reached. */
  target_profit_pct: number | null;
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
  recent_trades: Trade[];
}

export interface Trade {
  id: string;
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
  /** Long-term is split: this is the monthly allowance earmarked for mutual funds. */
  mf_budget: number;
  mf_available: number;
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
  daily_cap: number;
  /** The short-term scan's three filters: price floor, turnover floor, locked-band cut-off. */
  scan_min_price: number;
  scan_min_turnover: number;
  scan_locked_pct: number;
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

export interface RunRow {
  id: string;
  prompt: string;
  trigger: "schedule" | "change" | "manual";
  status: "running" | "done" | "failed";
  started_at: string;
  finished_at: string | null;
  summary: string | null;
  conversation_id: string | null;
  error: string | null;
}

export interface ArchivedInstrument {
  id: string;
  symbol: string;
  name: string;
  class: "stock" | "mutual_fund" | "ipo";
  fills: number;
  decisions: number;
  last_fill: string | null;
  /** What the round trip made, from the fills that were kept. There is no price to value it at. */
  realised: number;
  /** Money in minus money out: negative means the fills returned more than they took. */
  net_invested: number;
}

/** One scheduled prompt: what it is, when it is next due, and how its last run went. */
export interface ScheduledPrompt {
  prompt: string;
  label: string;
  rule: string;
  next_due_at: string | null;
  last: {
    status: "running" | "done" | "failed";
    trigger: "schedule" | "change" | "manual";
    at: string;
    finished_at: string | null;
    summary: string | null;
    error: string | null;
  } | null;
}

export interface CallRow {
  /** The uuid the server knows this call by. */
  id: string;
  ts: string;
  endpoint: string;
  /** Why the model was asked: "price moved …", "new headline …". */
  trigger: string | null;
  decision_id: string | null;
  decision_status: "pending" | "approved" | "rejected" | null;
  /** "BLOCKED" when the proposal could not be funded. */
  decision_action: string | null;
  /** Why it was refused, when it was. */
  decision_reason: string | null;
  /** The holding's name — what a person reads. The symbol is secondary. */
  instrument_name: string | null;
  instrument_symbol: string | null;
  /** How many moves a book call produced. Zero for every other kind of call. */
  plan_legs: number;
  /** The prompt template that was sent — decide_v1, plan_stock_v1, short_v1. */
  model: string | null;
  purpose: string | null;
  round: number | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  validator_verdict: string | null;
  error: string | null;
}

/** One call with its stored bodies. Fetched only when a row is opened. */
export interface CallDetail {
  id: string;
  ts: string;
  endpoint: string;
  purpose: string | null;
  round: number | null;
  trigger: string | null;
  /** Why a proposal was refused, shown inside View rather than under the pill. */
  decision_reason: string | null;
  instrument_name: string | null;
  instrument_symbol: string | null;
  prompt_template: string | null;
  completion_tokens: number | null;
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
    id: string;
    symbol: string;
    class: string;
    name: string;
    currency: string;
    timing_note: string;
    target_profit_pct: number | null;
    /** False when archived: the page offers a way back instead of another archive. */
    active: boolean;
  };
  /** The trailing year on the stored series; null when history does not reach back that far. */
  trailing_1y: number | null;
  /** The last 120 closes, for the chart. */
  bars: BarPoint[];
  /** Every bar the rules read — more than `bars` once history passes 120. */
  bar_count: number;
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
  headlines: { id: string; title: string; publisher: string | null; published_at: string; url: string }[];
  signals: SignalRow[];
  trades: Trade[];
}

export interface InstrumentDetail {
  instrument: {
    id: string;
    symbol: string;
    class: string;
    name: string;
    currency: string;
    timing_note: string;
    target_profit_pct: number | null;
    /** False when archived: the page offers a way back instead of another archive. */
    active: boolean;
  };
  headlines: { id: string; title: string; publisher: string | null; published_at: string; url: string }[];
  signals: SignalRow[];
}

/** The access token lives in memory only: a reload asks the refresh cookie for a new one. */
let accessToken: string | null = null;
let refreshing: Promise<boolean> | null = null;

export const auth = {
  token: () => accessToken,
  set: (token: string | null) => {
    accessToken = token;
  },
  /** One refresh at a time, shared by every call that hits a 401 together. */
  refresh: async (): Promise<boolean> => {
    if (refreshing) return refreshing;
    refreshing = (async () => {
      try {
        const r = await fetch("/api/auth/refresh", { method: "POST" });
        if (!r.ok) return false;
        const body = (await r.json()) as { access_token: string };
        accessToken = body.access_token;
        return true;
      } catch {
        return false;
      } finally {
        refreshing = null;
      }
    })();
    return refreshing;
  },
};


/** A request with the access token on it, refreshed once if the server says the token expired. */
async function sendRaw(path: string, init: RequestInit, retry = true): Promise<Response> {
  const headers: Record<string, string> = { ...((init.headers ?? {}) as Record<string, string>) };
  if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
  const r = await fetch(path, { ...init, headers });
  if (r.status === 401 && retry) {
    if (await auth.refresh()) return sendRaw(path, init, false);
    accessToken = null;
  }
  return r;
}

async function send<T>(path: string, init: RequestInit, retry: boolean): Promise<T> {
  const headers: Record<string, string> = { ...((init.headers ?? {}) as Record<string, string>) };
  if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
  const r = await fetch(path, { ...init, headers });
  if (r.status === 401 && retry && !path.startsWith("/api/auth/")) {
    if (await auth.refresh()) return send<T>(path, init, false);
    accessToken = null;
  }
  if (!r.ok) {
    const body = await r.text();
    throw new Error(`${path}: ${r.status} ${body.slice(0, 200)}`);
  }
  return r.json() as Promise<T>;
}

async function req<T>(path: string, init: RequestInit = {}): Promise<T> {
  return send<T>(path, init, true);
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
  const r = await sendRaw(`/api/dashboard?${q}`, { headers });
  if (r.status === 304) return { notModified: true };
  if (!r.ok) throw new Error(`dashboard: ${r.status}`);
  return {
    notModified: false,
    data: (await r.json()) as Dashboard,
    etag: r.headers.get("etag") ?? undefined,
  };
}

/** What the exchange is doing, and when that changes. */
export interface Market {
  state: "open" | "closed" | "holiday";
  reason: string;
  as_of: string;
  session: string;
  last_bar_at: string | null;
  /** When the exchange next opens; when it closes, if it is open now. */
  next_open_at: string | null;
  closes_at: string | null;
  holiday_calendar: boolean;
}

export interface McpCall {
  id: string;
  ts: string;
  key_name: string;
  key_id: string;
  prefix: string;
  tool: string;
  args_json: string;
  status: "ok" | "error";
  error: string | null;
  ms: number;
}

export interface SessionInfo {
  id: string;
  created_at: string;
  expires_at: string;
  user_agent: string;
  /** The token this browser is using right now. */
  current: boolean;
}

export interface ApiKey {
  id: string;
  name: string;
  scope: "read" | "write";
  prefix: string;
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
  revoked_at: string | null;
}

export const api = {
  /** Removes the instrument and every record against it; the cash its fills moved returns to the pot. */
  deleteInstrument: (symbol: string) =>
    req<{ symbol: string; deleted: boolean; cash: number }>(
      `/api/instruments/${encodeURIComponent(symbol)}/records`,
      { method: "DELETE" },
    ),
  /** Auth: the one door. Setup exists only while no admin does. */
  authStatus: () =>
    req<{ needs_setup: boolean; signed_in?: boolean; username?: string }>("/api/auth/status"),
  authSetup: (body: { username: string; password: string }) =>
    req<{ admin: { username: string }; access_token: string }>("/api/auth/setup", json(body)),
  authLogin: (body: { username: string; password: string }) =>
    req<{ admin: { username: string }; access_token: string }>("/api/auth/login", json(body)),
  authLogout: () => req<{ signed_out: boolean }>("/api/auth/logout", { method: "POST" }),
  changePassword: (body: { current: string; next: string }) =>
    req<{ changed: boolean; signed_out: boolean }>("/api/auth/password", json(body)),
  changeUsername: (body: { username: string; password: string }) =>
    req<{ username: string }>("/api/auth/username", json(body)),
  me: () => req<{ username: string }>("/api/auth/me"),

  /** The live browser sessions behind the refresh tokens. */
  sessions: () => req<{ sessions: SessionInfo[] }>("/api/sessions"),
  revokeAllSessions: () =>
    req<{ revoked: number; signed_out: boolean }>("/api/sessions/revoke-all", { method: "POST" }),

  /** MCP access keys; the plaintext comes back once, from createKey. */
  keys: () => req<{ keys: ApiKey[] }>("/api/keys"),
  createKey: (body: { name: string; scope: "read" | "write"; days: number | null }) =>
    req<{ key: ApiKey; token: string }>("/api/keys", json(body)),
  revokeKey: (id: string) => req<{ id: string; revoked: boolean }>(`/api/keys/${id}`, { method: "DELETE" }),
  /** What an assistant asked the book to do, newest first; the server caps it at 5000. */
  mcpCalls: (limit = 200) => req<{ calls: McpCall[] }>(`/api/mcp/calls?limit=${limit}`),
  dashboard: (
    mode: Mode = "long",
    decisions: "pending" | "approved" | "rejected" | "all" = "pending",
  ) =>
    req<Dashboard>(
      `/api/dashboard?mode=${mode}${decisions === "pending" ? "" : `&decisions=${decisions}`}`,
    ),
  settings: () => req<Settings>("/api/settings"),
  testProxy: () =>
    req<{ ok: boolean; status: number | null; millis: number; error: string | null }>(
      "/api/settings/proxy/test",
      { method: "POST" },
    ),
  /** Runs: what the scheduler started, opened before the work and closed after it. */
  runs: (limit = 25) =>
    req<{ runs: RunRow[]; running: number }>(`/api/runs?limit=${limit}`),
  /** Whether the exchange is trading right now, and why. */
  market: () => req<Market>("/api/market"),
  /** The schedule: each prompt's last run and when it is next due, computed by the server. */
  schedule: () =>
    req<{ now: string; prompts: ScheduledPrompt[] }>("/api/schedule"),
  runNow: (prompt: string, symbol?: string) =>
    req<{ id: string; status: string }>("/api/runs", json({ prompt, symbol })),
  /** The values the audit filters offer, straight from the record. */
  callFacets: () =>
    req<{ prompts: string[]; calls: string[]; models: string[] }>("/api/calls/facets"),
  /** A window into the audit list; the total comes back in a header. */
  calls: async (
    offset = 0,
    limit = 25,
    filters: { prompt?: string; channel?: string; model?: string; review?: string } = {},
  ) => {
    const q = new URLSearchParams({ offset: String(offset), limit: String(limit) });
    for (const [k, v] of Object.entries(filters)) {
      if (!v) continue;
      q.set(k === "channel" ? "call" : k, v);
    }
    const r = await sendRaw(`/api/calls?${q}`, {});
    if (!r.ok) throw new Error(`/api/calls: ${r.status} ${(await r.text()).slice(0, 200)}`);
    const rows = (await r.json()) as CallRow[];
    const total = Number(r.headers.get("x-total-count") ?? rows.length);
    return { rows, total, offset, limit };
  },
  archived: () => req<ArchivedInstrument[]>("/api/archived"),
  restoreInstrument: (symbol: string) =>
    req<{ symbol: string; restored: boolean }>(
      `/api/instruments/${encodeURIComponent(symbol)}/restore`,
      { method: "POST" },
    ),
  approveSignal: (
    id: string,
    body: {
      qty?: number;
      price?: number;
      at?: string;
      /** A plan carries one fill per leg. */
      fills?: { id: string; qty: number; price: number }[];
    },
  ) =>
    req<{ id: string; status: string; recorded: boolean }>(
      `/api/signals/${id}/approve`,
      json(body),
    ),
  deleteTrade: (id: string) =>
    req<{ id: string; deleted: boolean }>(`/api/trades/${id}`, { method: "DELETE" }),
  editTrade: (id: string, body: { qty?: number; price?: number; at?: string }) =>
    req<{ id: string; edited: boolean }>(`/api/trades/${id}`, {
      ...json(body),
      method: "PATCH",
    }),
  rejectSignal: (id: string, reason?: string) =>
    req<{ id: string; status: string }>(`/api/signals/${id}/reject`, json({ reason })),
  refreshInstrument: (symbol: string) =>
    // The server answers `{bars, headlines}`; the old name said `news` and printed "undefined".
    req<{ ok: boolean; bars: number; headlines: number; cached: boolean }>(
      `/api/instruments/${encodeURIComponent(symbol)}/refresh`,
      { method: "POST" },
    ),
  archiveInstrument: (symbol: string) =>
    req<{ symbol: string; archived: boolean }>(
      `/api/instruments/${encodeURIComponent(symbol)}`,
      { method: "DELETE" },
    ),
  callDetail: (id: string) => req<CallDetail>(`/api/calls/${id}`),
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
    const r = await sendRaw("/api/notify/test", { method: "POST" });
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
      Pick<Settings, "budget" | "currency" | "trading_budget" | "mf_budget" | "ntfy_url" | "ntfy_topic" | "llm_base_url" | "llm_model" | "llm_provider">
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
      scan_min_price?: number;
      scan_min_turnover?: number;
      scan_locked_pct?: number;
      daily_cap?: number;
      proxy?: string;
      proxy_user?: string;
      proxy_pass?: string;
      proxy_bypass?: string;
    },
  ) => {
    const r = await sendRaw("/api/settings", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    });
    return { ok: r.ok, status: r.status, body: await r.json() };
  },
};
