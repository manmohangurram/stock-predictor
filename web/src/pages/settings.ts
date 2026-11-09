import type { Settings } from "../api";
import { escapeHtml, money } from "../format";
import { renderApp, renderHead } from "../components/shell";
import { TABS, tabRail, type Tab } from "./settings-tabs";

export { isTab, TABS, type Tab } from "./settings-tabs";

/** The one rule the form enforces client-side; the server enforces it again. */
export const shouldDisableSave = (budget: number, committed: number): boolean =>
  !Number.isFinite(budget) || budget < committed;

/**
 * Edit drafts survive a tab switch, so leaving a tab never silently discards typing. A tab is
 * cleared by stamping it or by reverting it.
 */
const drafts = new Map<Tab, Record<string, string>>();

export function setDraft(tab: Tab, name: string, value: string): void {
  drafts.set(tab, { ...(drafts.get(tab) ?? {}), [name]: value });
}

export function clearDraft(tab: Tab): void {
  drafts.delete(tab);
}

export const draftOf = (tab: Tab, name: string): string | undefined => drafts.get(tab)?.[name];

export const tabHasDraft = (tab: Tab): boolean => {
  const d = drafts.get(tab);
  return !!d && Object.keys(d).length > 0;
};

/** A field, with its value taken from the draft when one exists. */
function field(
  tab: Tab,
  name: string,
  label: string,
  value: string | number,
  opts: { type?: string; step?: string; help?: string; placeholder?: string; unit?: string } = {},
): string {
  const raw = draftOf(tab, name) ?? String(value ?? "");
  const type = opts.type ?? "text";
  return `<div class="field">
    <label class="lbl" for="${name}">${label}</label>
    <div class="field-line">
      <input id="${name}" name="${name}" type="${type}"
             ${opts.step ? `step="${opts.step}"` : ""}
             ${opts.placeholder ? `placeholder="${escapeHtml(opts.placeholder)}"` : ""}
             value="${escapeHtml(raw)}" autocomplete="off" />
      ${opts.unit ? `<span class="unit">${opts.unit}</span>` : ""}
    </div>
    ${opts.help ? `<p class="help">${opts.help}</p>` : ""}
  </div>`;
}

function select(tab: Tab, name: string, label: string, value: string, options: [string, string][], help = "") {
  const current = draftOf(tab, name) ?? value;
  return `<div class="field">
    <label class="lbl" for="${name}">${label}</label>
    <select id="${name}" name="${name}">
      ${options
        .map(([v, text]) => `<option value="${v}" ${v === current ? "selected" : ""}>${text}</option>`)
        .join("")}
    </select>
    ${help ? `<p class="help">${help}</p>` : ""}
  </div>`;
}

/** Seconds as a person would say them: 900 → "15 minutes", 7200 → "2 hours". */
export function humanDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs <= 0) return "—";
  const s = Math.round(secs);
  if (s < 60) return `${s} second${s === 1 ? "" : "s"}`;
  const minutes = s / 60;
  if (minutes < 60) {
    const m = Math.round(minutes * 10) / 10;
    return `${m} minute${m === 1 ? "" : "s"}`;
  }
  const hours = Math.floor(minutes / 60);
  const rest = Math.round(minutes - hours * 60);
  if (rest === 0) return `${hours} hour${hours === 1 ? "" : "s"}`;
  return `${hours} h ${rest} min`;
}

/** A line of help that recomputes itself as the field is typed. */
function derived(name: string, value: number, template: string): string {
  const shown = template.replace("{v}", humanDuration(value));
  return `<span class="derived" data-seconds="${name}" data-template="${escapeHtml(template)}">${escapeHtml(shown)}</span>`;
}

const MONEY = (s: Settings, tab: Tab): string => {
  const below = shouldDisableSave(s.budget, s.committed);
  return `
    ${field(tab, "budget", "Monthly allowance, long-term", Math.round(s.budget), {
      type: "number",
      step: "1",
      unit: s.currency,
      help: `Granted on the 1st of each month. Unspent allowance expires at month end — it is not carried over, and it is not counted as a trading loss.`,
    })}
    ${field(tab, "trading_budget", "Monthly allowance, trading", Math.round(s.trading_budget), {
      type: "number",
      step: "1",
      unit: s.currency,
      help: `A separate monthly allowance for the short-term pot, ${money(s.trading_available, 0)} left this month. Neither pot draws on the other.`,
    })}
    ${select(tab, "currency", "Currency", s.currency, ["INR", "USD", "EUR", "GBP"].map((c) => [c, c]),
      s.currency_locked
        ? "Currency is fixed once the ledger has entries."
        : "Locked permanently after the first recorded transaction.")}
    ${below ? `<p class="guard">Committed is ${money(s.committed, 0)} — the budget cannot go below it.</p>` : ""}
  `;
};

const LIMIT_ROWS: [keyof Settings["limits_long"], string, string, string][] = [
  ["min_profit_pct", "Minimum profit", "0.5", "%"],
  ["max_profit_pct", "Maximum profit", "0.5", "%"],
  ["min_trade_amount", "Smallest trade", "1", ""],
  ["max_trade_amount", "Largest trade", "1", ""],
  ["max_instrument_pct", "One stock may be", "1", "%"],
];

const RISK = (s: Settings, tab: Tab): string => `
  <table class="limits-table">
    <thead><tr><th scope="col">Limit</th>
      <th scope="col">Long-term limits</th><th scope="col">Short-term limits</th></tr></thead>
    <tbody>
      ${LIMIT_ROWS.map(([key, label, step, unit]) => {
        const cell = (mode: "long" | "trade") => {
          const name = `${mode}_${key}`;
          const value = draftOf(tab, name) ?? String(s[`limits_${mode}`][key]);
          return `<td><input id="${name}" name="${name}" type="number" min="0" step="${step}"
                      value="${escapeHtml(value)}" aria-label="${label}, ${mode === "long" ? "long-term" : "trading"}" /></td>`;
        };
        return `<tr><th scope="row">${label} ${unit}</th>${cell("long")}${cell("trade")}</tr>`;
      }).join("")}
    </tbody>
  </table>
  <p class="help">
    A take-profit below this is rejected, and so is a target above the maximum: no fantasy numbers.
    Percentages are of that pot, so the caps keep their meaning when the budget changes;
    a stock's own profit expectation must fall inside the band, and the band's maximum always wins.
    Amounts are in ${s.currency}.
  </p>
  <p class="help">
    A smallest trade below this is not worth its fees; a largest trade caps one fill. One stock may
    be at most its share of the pot.
  </p>
`;

const MODEL = (s: Settings, tab: Tab): string => `
  ${select(tab, "llm_provider", "Provider", s.llm_provider, [
    ["deepseek", "DeepSeek"],
    ["openai", "OpenAI"],
    ["gemini", "Google Gemini"],
    ["custom", "OpenAI-compatible (OpenRouter, Groq, Ollama…)"],
  ])}
  ${field(tab, "llm_model", "Model", s.llm_model, {
    placeholder: "default for the provider",
    help: "Leave blank to use the provider's default.",
  })}
  ${field(tab, "llm_base_url", "Base URL", s.llm_base_url, {
    placeholder: "default for the provider",
    help: "Change it for a proxy or a local Ollama: <code>http://localhost:11434/v1</code>.",
  })}
  ${field(tab, "llm_key", "API key", "", {
    type: "password",
    placeholder: s.llm_key_configured ? "•••••••• configured — leave blank to keep" : "key for the selected provider",
    help: s.llm_key_configured
      ? "Configured. The key is never shown again, here or over the API."
      : "Without a key the rules run alone and no model call is made.",
  })}
`;

const NOTIFICATIONS = (s: Settings, tab: Tab): string => `
  ${field(tab, "ntfy_topic", "Topic", s.ntfy_topic, {
    placeholder: "stock-predicter-7f3k9m2q",
    help: "Subscribe to this topic in the ntfy app. Anyone who knows the name can read it.",
  })}
  ${field(tab, "ntfy_url", "Server", s.ntfy_url, {
    placeholder: "https://ntfy.sh",
    help: "Use your own ntfy instance here if you self-host one.",
  })}
  ${field(tab, "ntfy_token", "Access token", "", {
    type: "password",
    placeholder: s.ntfy_token_configured ? "•••••••• configured — leave blank to keep" : "tk_… (only for a protected topic)",
    help: s.ntfy_token_configured
      ? "Configured. The token is never shown again, here or over the API."
      : "Optional. Needed only when the topic requires authentication.",
  })}
  <div class="row">
    <button class="btn" type="button" id="test-notify">Send a test notification</button>
    <span class="help" id="notify-status" role="status"></span>
  </div>
`;

const CADENCE = (s: Settings, tab: Tab): string => `
  ${field(tab, "poll_secs", "Poll while the market is open", s.runtime.poll_secs, {
    type: "number", step: "60", unit: "seconds",
    help: `09:00–16:00 IST, Monday to Friday — ${derived("poll_secs", s.runtime.poll_secs, "wakes every {v}")}.`,
  })}
  ${field(tab, "off_hours_secs", "Poll when the market is closed", s.runtime.off_hours_secs, {
    type: "number", step: "60", unit: "seconds",
    help: `Outside that window, only to catch headlines — ${derived("off_hours_secs", s.runtime.off_hours_secs, "wakes every {v}")}.`,
  })}
  ${field(tab, "prompt_window_hours", "Tape in each prompt", s.runtime.prompt_window_hours, {
    type: "number", step: "1", unit: "hours",
    help: "Hours of 15-minute bars the model sees — the biggest lever on token cost.",
  })}
  ${field(tab, "daily_cap", "Model calls per day, at most", s.runtime.daily_cap, {
    type: "number", step: "1", unit: "calls",
    help: "Past it, the rules run alone for the rest of the day.",
  })}
  ${field(tab, "min_price_move", "Call only if the price moved", s.runtime.min_price_move, {
    type: "number", step: "0.5", unit: s.currency,
    help: "Since the last call. A new headline always earns a call regardless.",
  })}
  ${field(tab, "max_silence_secs", "…or after silence of", s.runtime.max_silence_secs, {
    type: "number", step: "60", unit: "seconds",
    help: `0 means a call is only for a change: no price move and no new headline, no call. ${derived("max_silence_secs", s.runtime.max_silence_secs, "Set above zero to also call every {v}.")}`,
  })}
`;

const NETWORK = (s: Settings, tab: Tab): string => `
  ${field(tab, "proxy", "Proxy address", "", {
    placeholder: s.runtime.proxy_configured ? escapeHtml(s.runtime.proxy_host) : "http://10.0.0.2:3128",
    help: `Host and port only, no credentials. Leave blank to clear the proxy and connect directly.
           The environment's <code>HTTP_PROXY</code> is never used.`,
  })}
  ${field(tab, "proxy_user", "Proxy username", s.runtime.proxy_user ?? "", {
    placeholder: "optional",
    help: "Sent as basic proxy authentication, together with the password below.",
  })}
  ${field(tab, "proxy_pass", "Proxy password", "", {
    type: "password",
    placeholder: s.runtime.proxy_pass_configured ? "leave blank to keep" : "stored encrypted",
    help: s.runtime.proxy_pass_configured
      ? "Configured. The password is never shown again, here or over the API."
      : "Encrypted at rest with the same key as the model API key.",
  })}
  ${field(tab, "proxy_bypass", "Bypass the proxy for", s.runtime.proxy_bypass ?? "", {
    placeholder: "localhost, 127.0.0.1",
    help: "Comma-separated hosts that connect directly.",
  })}
  <div class="row">
    <button class="btn" type="button" id="test-proxy">Test the proxy</button>
    <span class="help" id="proxy-status" role="status"></span>
  </div>
`;

const PANES: Record<Tab, (s: Settings, t: Tab) => string> = {
  money: MONEY,
  risk: RISK,
  model: MODEL,
  notifications: NOTIFICATIONS,
  cadence: CADENCE,
  network: NETWORK,
};

function pane(s: Settings, tab: Tab): string {
  const spec = TABS.find((t) => t.id === tab)!;
  const dirty = tabHasDraft(tab);
  return `<main class="settings-pane" role="tabpanel" id="panel-${tab}" aria-labelledby="tab-${tab}" tabindex="0">
    <div class="pane-head">
      <h2>${spec.title}</h2>
      <span class="pill ${dirty ? "todo" : "ok"}">${dirty ? "unsaved" : "saved"}</span>
    </div>
    <p class="lede">${spec.lede}</p>
    <form id="tab-form" data-tab="${tab}" autocomplete="off">
      ${PANES[tab](s, tab)}
      <div class="pane-foot">
        <span class="note" id="tab-status" role="status"></span>
        <div class="row">
          <button class="btn ghost" type="button" id="revert">Revert</button>
          <button class="btn" type="submit">Stamp</button>
        </div>
      </div>
    </form>
  </main>`;
}

export function renderSettings(s: Settings, tab: Tab = "money"): string {
  return renderApp(
    "settings",
    `${renderHead("Settings", "Every value is a database row, applied on the next tick", [
      { label: "Long-term", value: money(s.budget, 0) },
      { label: "Short-term", value: money(s.trading_budget, 0) },
      { label: "Free", value: money(s.available, 0) },
    ])}
    <div class="tabs-row">${tabRail(tab, s)}</div>
    ${pane(s, tab)}`,
    { label: "Free to deploy", value: money(s.available, 0) },
  );
}
