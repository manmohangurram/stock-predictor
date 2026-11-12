import { api, type Settings, type Limits } from "../api";
import { clearDraft, humanDuration, rupees, setDraft, type Tab } from "../pages/settings";

function readLimits(data: FormData, mode: "long" | "trade") {
  const num = (name: string) => Number(data.get(`${mode}_${name}`) ?? 0);
  return {
    min_profit_pct: num("min_profit_pct"),
    max_profit_pct: num("max_profit_pct"),
    min_trade_amount: num("min_trade_amount"),
    max_trade_amount: num("max_trade_amount"),
    max_instrument_pct: num("max_instrument_pct"),
  };
}

/// The settings form: derived help text, a dirty badge, and a save that only patches what changed.
export function wireSettings(
  tab: Tab,
  s: Settings,
  reload: () => Promise<void>,
): void {
  const form = document.querySelector<HTMLFormElement>("#tab-form");
  const status = document.querySelector<HTMLElement>("#tab-status");

  // Help text that recomputes as the field is typed: "900 seconds" reads "every 15 minutes".
  const refreshDerived = () => {
    form?.querySelectorAll<HTMLElement>("[data-seconds]").forEach((el) => {
      const input = form?.querySelector<HTMLInputElement>(`[name="${el.dataset.seconds}"]`);
      if (!input) return;
      const text = (el.dataset.template ?? "{v}").replace("{v}", humanDuration(Number(input.value)));
      el.textContent = text;
    });
  };
  form?.addEventListener("input", refreshDerived);

  // Drafts live in the settings module, so switching tabs keeps what was typed.
  form?.addEventListener("input", (ev) => {
    const el = ev.target as HTMLInputElement | HTMLSelectElement;
    if (!el.name) return;
    setDraft(tab, el.name, el.value);
    document.querySelector<HTMLElement>(".pane-head .pill")?.replaceChildren("unsaved");
    document.querySelector<HTMLElement>(".pane-head .pill")?.setAttribute("class", "pill todo");
  });

  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const text = (name: string) => String(data.get(name) ?? "").trim();
    const num = (name: string) => Number(data.get(name));
    let patch: Parameters<typeof api.putSettings>[0];

    switch (tab) {
      case "money": {
        const budget = num("budget");
        patch = {
          budget,
          trading_budget: num("trading_budget"),
          mf_budget: num("mf_budget"),
          currency: text("currency"),
        };
        break;
      }
      case "risk":
        patch = { limits_long: readLimits(data, "long"), limits_trade: readLimits(data, "trade") };
        break;
      case "model": {
        const key = text("llm_key");
        patch = {
          llm_provider: text("llm_provider"),
          llm_model: text("llm_model"),
          llm_base_url: text("llm_base_url"),
          ...(key ? { llm_key: key } : {}),
        };
        break;
      }
      case "notifications": {
        const token = text("ntfy_token");
        patch = {
          ntfy_topic: text("ntfy_topic"),
          ntfy_url: text("ntfy_url"),
          ...(token ? { ntfy_token: token } : {}),
        };
        break;
      }
      case "cadence":
        patch = {
          poll_secs: num("poll_secs"),
          off_hours_secs: num("off_hours_secs"),
          prompt_window_hours: num("prompt_window_hours"),
          daily_cap: num("daily_cap"),
          min_price_move: num("min_price_move"),
          scan_min_price: num("scan_min_price"),
          // Typed in crore, stored in rupees.
          scan_min_turnover:
            num("scan_min_turnover") === undefined
              ? undefined
              : rupees(num("scan_min_turnover") ?? 0),
          scan_locked_pct: num("scan_locked_pct"),
        };
        break;
      case "network": {
        const pass = text("proxy_pass");
        patch = {
          proxy: text("proxy"),
          proxy_user: text("proxy_user"),
          proxy_bypass: text("proxy_bypass"),
          ...(pass ? { proxy_pass: pass } : {}),
        };
        break;
      }
    }

    if (status) status.textContent = "Stamping…";
    const res = await api.putSettings(patch);
    if (!res.ok) {
      // The error belongs on this tab, next to the field that caused it — not a toast.
      if (status) status.textContent = String(res.body.error ?? "rejected");
      if (tab === "money" && res.body.error === "below_committed") {
        if (status) status.textContent = `Committed is ${res.body.committed} — the budget cannot go below it.`;
      }
      return;
    }
    clearDraft(tab);
    await reload();
  });

  document.querySelector<HTMLButtonElement>("#revert")?.addEventListener("click", async () => {
    clearDraft(tab);
    await reload();
  });

  document.querySelector<HTMLButtonElement>("#test-notify")?.addEventListener("click", async (ev) => {
    const button = ev.currentTarget as HTMLButtonElement;
    const el = document.querySelector<HTMLElement>("#notify-status");
    button.disabled = true;
    if (el) el.textContent = "Sending…";
    const res = await api.testNotify();
    button.disabled = false;
    if (el) {
      el.textContent = res.ok
        ? "Sent. Check your phone."
        : `Failed: ${res.body.error ?? "unknown error"}`;
    }
  });

  document.querySelector<HTMLButtonElement>("#test-proxy")?.addEventListener("click", async (ev) => {
    const button = ev.currentTarget as HTMLButtonElement;
    const el = document.querySelector<HTMLElement>("#proxy-status");
    button.disabled = true;
    if (el) el.textContent = "Sending one request through it…";
    try {
      const r = await api.testProxy();
      if (el) {
        el.textContent = r.ok
          ? `Reached the provider · HTTP ${r.status} · ${r.millis} ms`
          : `Failed after ${r.millis} ms — ${r.error}`;
      }
    } catch (e) {
      if (el) el.textContent = String(e);
    }
    button.disabled = false;
  });

  // Arrow keys move between tabs, as a tablist should.
  document.querySelectorAll<HTMLAnchorElement>('.settings-rail [role="tab"]').forEach((el, i, all) => {
    el.addEventListener("keydown", (ev) => {
      const step = ev.key === "ArrowRight" ? 1 : ev.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      ev.preventDefault();
      const next = all[(i + step + all.length) % all.length];
      next.focus();
      location.href = next.getAttribute("href")!;
    });
  });
}
