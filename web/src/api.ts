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
  currency_locked: boolean;
  positions: PositionRow[];
  signals: SignalRow[];
  blocked: SignalRow[];
}

export interface Settings {
  budget: number;
  committed: number;
  available: number;
  currency: string;
  currency_locked: boolean;
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

async function get<T>(path: string): Promise<T> {
  const r = await fetch(path);
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return r.json() as Promise<T>;
}

export const api = {
  dashboard: () => get<Dashboard>("/api/dashboard"),
  settings: () => get<Settings>("/api/settings"),
  calls: () => get<CallRow[]>("/api/calls"),
  putSettings: async (patch: Partial<Pick<Settings, "budget" | "currency">>) => {
    const r = await fetch("/api/settings", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    });
    return { ok: r.ok, status: r.status, body: await r.json() };
  },
};
