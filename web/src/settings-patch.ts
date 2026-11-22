/**
 * What a tab sends. Pure, so the shape can be read without a browser: numbers are parsed, the
 * turnover floor is converted from crore, and a secret is only included when one was typed — an
 * empty box means "keep the stored key", never "clear it".
 */
import type { Settings } from "./api";

export type Tab = "money" | "risk" | "model" | "notifications" | "cadence" | "network";

const LAKH = 100_000;

const num = (v: string | undefined): number => Number(v ?? 0);
const text = (v: string | undefined): string => String(v ?? "").trim();

function limits(values: Record<string, string>, mode: "long" | "trade") {
  return {
    min_profit_pct: num(values[`${mode}_min_profit_pct`]),
    max_profit_pct: num(values[`${mode}_max_profit_pct`]),
    min_trade_amount: num(values[`${mode}_min_trade_amount`]),
    max_trade_amount: num(values[`${mode}_max_trade_amount`]),
    max_instrument_pct: num(values[`${mode}_max_instrument_pct`]),
  };
}

export type PatchResult =
  | { patch: Parameters<typeof import("./api").api.putSettings>[0] }
  | { error: string };

export function patchFor(tab: Tab, values: Record<string, string>): PatchResult {
  switch (tab) {
    case "money":
      return {
        patch: {
          budget: num(values.budget),
          trading_budget: num(values.trading_budget),
          mf_budget: num(values.mf_budget),
          currency: text(values.currency),
        },
      };
    case "risk":
      return { patch: { limits_long: limits(values, "long"), limits_trade: limits(values, "trade") } };
    case "model": {
      const key = text(values.llm_key);
      return {
        patch: {
          llm_provider: text(values.llm_provider),
          llm_model: text(values.llm_model),
          llm_base_url: text(values.llm_base_url),
          ...(key ? { llm_key: key } : {}),
        },
      };
    }
    case "notifications": {
      const token = text(values.ntfy_token);
      return {
        patch: {
          ntfy_topic: text(values.ntfy_topic),
          ntfy_url: text(values.ntfy_url),
          ...(token ? { ntfy_token: token } : {}),
        },
      };
    }
    case "cadence": {
      const lakh = num(values.scan_min_turnover);
      if (!Number.isInteger(lakh)) {
        return { error: "Turnover floor: enter a whole number of lakh (1 lakh is 1,00,000; 100 lakh is 1 crore)." };
      }
      return {
        patch: {
          poll_secs: num(values.poll_secs),
          off_hours_secs: num(values.off_hours_secs),
          prompt_window_hours: num(values.prompt_window_hours),
          daily_cap: num(values.daily_cap),
          min_price_move: num(values.min_price_move),
          scan_min_price: num(values.scan_min_price),
          scan_min_turnover: Math.round(lakh * LAKH),
          scan_locked_pct: num(values.scan_locked_pct),
        },
      };
    }
    case "network": {
      const pass = text(values.proxy_pass);
      return {
        patch: {
          proxy: text(values.proxy),
          proxy_user: text(values.proxy_user),
          proxy_bypass: text(values.proxy_bypass),
          ...(pass ? { proxy_pass: pass } : {}),
        },
      };
    }
  }
}

/** The values a tab starts from, so a discard is a re-seed rather than a reload. */
export function seed(tab: Tab, s: Settings): Record<string, string> {
  const v: Record<string, string> = {};
  switch (tab) {
    case "money":
      return {
        budget: String(Math.round(s.budget)),
        mf_budget: String(Math.round(s.mf_budget)),
        trading_budget: String(Math.round(s.trading_budget)),
        currency: s.currency,
      };
    case "risk":
      for (const mode of ["long", "trade"] as const) {
        for (const [k, val] of Object.entries(s[`limits_${mode}`])) v[`${mode}_${k}`] = String(val);
      }
      return v;
    case "model":
      return {
        llm_provider: s.llm_provider,
        llm_model: s.llm_model,
        llm_base_url: s.llm_base_url,
        llm_key: "",
      };
    case "notifications":
      return { ntfy_topic: s.ntfy_topic, ntfy_url: s.ntfy_url, ntfy_token: "" };
    case "cadence":
      return {
        poll_secs: String(s.runtime.poll_secs),
        off_hours_secs: String(s.runtime.off_hours_secs),
        prompt_window_hours: String(s.runtime.prompt_window_hours),
        daily_cap: String(s.runtime.daily_cap),
        min_price_move: String(s.runtime.min_price_move),
        scan_min_price: String(s.runtime.scan_min_price),
        scan_min_turnover: String(s.runtime.scan_min_turnover / LAKH),
        scan_locked_pct: String(s.runtime.scan_locked_pct),
      };
    case "network":
      return { proxy: "", proxy_user: s.runtime.proxy_user ?? "", proxy_bypass: s.runtime.proxy_bypass ?? "", proxy_pass: "" };
  }
}
