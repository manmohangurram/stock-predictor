import { api, dashboardCached, type Hit, type Settings } from "./api";
import { renderCalls, renderExchangeDetail } from "./pages/calls";
import { qtyLimit, renderInstrument } from "./pages/instrument";
import { renderLedger } from "./pages/ledger";
import {
  clearDraft,
  isTab,
  renderSettings,
  setDraft,
  humanDuration,
  shouldDisableSave,
  TABS,
  type Tab,
} from "./pages/settings";

const app = document.querySelector<HTMLDivElement>("#app")!;
let dashboardEtag: string | null = null;

function failure(e: unknown): string {
  return `<div class="shell"><div class="error-band">Something failed: ${String(e)}
    <button class="btn" onclick="location.reload()">Retry</button></div></div>`;
}

async function route(): Promise<void> {
  const path = window.location.pathname;
  try {
    if (path === "/settings" || path.startsWith("/settings/")) {
      const wanted = path.startsWith("/settings/")
        ? decodeURIComponent(path.slice("/settings/".length))
        : "money";
      const tab: Tab = isTab(wanted) ? wanted : "money";
      if (tab !== wanted) history.replaceState(null, "", `/settings/${tab}`);
      const s = await api.settings();
      app.innerHTML = renderSettings(s, tab);
      wireSettings(tab, s);
    } else if (path === "/calls") {
      app.innerHTML = renderCalls(await api.calls());
      wireCallRows();
    } else if (path.startsWith("/i/")) {
      const symbol = decodeURIComponent(path.slice(3));
      // One request: the analysis payload carries the instrument, its signals and its fills.
      const analysis = await api.analysis(symbol);
      app.innerHTML = renderInstrument(analysis, analysis, analysis.trades);
      wireTradeForm(symbol);
      wireDecisions();
      wireDecisionActions();
      wireTargetProfit(symbol);
    } else {
      const mode = (new URLSearchParams(location.search).get("mode") as
        | "long"
        | "trade"
        | null) ?? "long";
      // One request: the dashboard payload carries the rail's recent fills.
      const decisions = (new URLSearchParams(location.search).get("decisions") as
        | "pending"
        | "approved"
        | "rejected"
        | "all"
        | null) ?? "pending";
      const res = await dashboardCached(mode, dashboardEtag, decisions);
      if (res.notModified) return; // nothing changed: keep the DOM, and the user's open rows
      dashboardEtag = res.etag ?? null;
      const d = res.data!;
      app.innerHTML = renderLedger(d, d.recent_trades);
      // The nav belongs inside the content column, not floating in the page margin.
      wireRows();
      wireSearch();
      wireDecisionActions();
    }
  } catch (e) {
    app.innerHTML = failure(e);
  }
}


function wireRows(): void {
  document.querySelectorAll<HTMLElement>("[data-symbol]").forEach((row) => {
    const go = () => {
      const symbol = row.dataset.symbol;
      if (symbol) window.location.href = `/i/${encodeURIComponent(symbol)}`;
    };
    row.addEventListener("click", go);
    row.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        go();
      }
    });
  });
}

function wireSearch(): void {
  const input = document.querySelector<HTMLInputElement>("#search");
  const box = document.querySelector<HTMLDivElement>("#results");
  if (!input || !box) return;

  let timer: number | undefined;
  let last: Hit[] = [];

  const draw = (hits: Hit[]) => {
    box.innerHTML = hits.length
      ? hits
          .map(
            (h, i) => `<button class="hit" type="button" data-i="${i}" role="option"
            aria-label="Add ${h.code}">
            <b class="num">${h.code}</b>
            <span>${h.name}</span>
            <em>${h.venue || h.class}</em>
          </button>`,
          )
          .join("")
      : input.value.trim().length >= 2
        ? `<p class="empty">Nothing found.</p>`
        : "";
    box.querySelectorAll<HTMLButtonElement>(".hit").forEach((b) => {
      b.addEventListener("click", async () => {
        const hit = last[Number(b.dataset.i)];
        if (!hit) return;
        b.disabled = true;
        b.textContent = `Adding ${hit.code}…`;
        try {
          await api.addInstrument(hit.code, hit.name, hit.class);
          window.location.href = `/i/${encodeURIComponent(hit.code)}`;
        } catch (e) {
          b.textContent = `Failed: ${String(e)}`;
        }
      });
    });
  };

  input.addEventListener("input", () => {
    window.clearTimeout(timer);
    const q = input.value.trim();
    if (q.length < 2) {
      last = [];
      draw([]);
      return;
    }
    timer = window.setTimeout(async () => {
      try {
        last = await api.search(q);
        draw(last);
      } catch {
        box.innerHTML = `<p class="empty">Search unavailable.</p>`;
      }
    }, 250);
  });
}

/** Approve posts the fill you type; reject records a reason. Both stay on the page. */
function wireDecisionActions(): void {
  // Both the ledger and the instrument page render the same actions, so this works on either.
  const card = document.body;
  if (!card.querySelector("[data-approve]")) return;
  const field = (id: string, name: string) =>
    card.querySelector<HTMLInputElement>(`[data-approve-form="${id}"] [name="${name}"]`);
  const status = (id: string) => card.querySelector<HTMLElement>(`[data-status="${id}"]`);

  card.querySelectorAll<HTMLButtonElement>("[data-approve]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.approve!;
      const form = card.querySelector<HTMLFormElement>(`[data-approve-form="${id}"]`);
      if (!form) return;
      form.hidden = !form.hidden;
      button.setAttribute("aria-expanded", String(!form.hidden));
    });
  });
  card.querySelectorAll<HTMLButtonElement>("[data-reject]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.reject!;
      const form = card.querySelector<HTMLFormElement>(`[data-reject-form="${id}"]`);
      if (!form) return;
      form.hidden = !form.hidden;
    });
  });
  card.querySelectorAll<HTMLButtonElement>("[data-cancel]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.cancel!;
      const approve = card.querySelector<HTMLFormElement>(`[data-approve-form="${id}"]`);
      const reject = card.querySelector<HTMLFormElement>(`[data-reject-form="${id}"]`);
      if (approve) approve.hidden = true;
      if (reject) reject.hidden = true;
    });
  });
  card.querySelectorAll<HTMLFormElement>("[data-approve-form]").forEach((form) => {
    form.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const id = Number(form.dataset.approveForm);
      const el = status(String(id));
      if (el) el.textContent = "Posting…";
      try {
        await api.approveSignal(id, {
          qty: Number(field(String(id), "qty")?.value),
          price: Number(field(String(id), "price")?.value),
          at: field(String(id), "at")?.value || undefined,
        });
        await route();
      } catch (e) {
        if (el) el.textContent = String(e);
      }
    });
  });
  card.querySelectorAll<HTMLFormElement>("[data-reject-form]").forEach((form) => {
    form.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const id = Number(form.dataset.rejectForm);
      const reason = form.querySelector<HTMLInputElement>('[name="reason"]')?.value ?? "";
      const el = status(String(id));
      if (el) el.textContent = "Rejecting…";
      try {
        await api.rejectSignal(id, reason || undefined);
        await route();
      } catch (e) {
        if (el) el.textContent = String(e);
      }
    });
  });
}

function wireTradeForm(symbol: string): void {
  const form = document.querySelector<HTMLFormElement>(".trade-form");
  const status = document.querySelector<HTMLElement>("#trade-status");
  const sideInputs = form ? Array.from(form.querySelectorAll<HTMLInputElement>('input[name="side"]')) : [];
  const sideValue = (): "buy" | "sell" =>
    (sideInputs.find((r) => r.checked)?.value as "buy" | "sell") ?? "buy";
  const qtyInput = form?.querySelector<HTMLInputElement>('input[name="qty"]');

  // Cap the quantity only while selling. Applying it to a buy made the browser reject any
  // purchase larger than the current holding before the request was ever sent.
  const syncLimit = () => {
    if (!qtyInput) return;
    const held = qtyInput.dataset.held ? Number(qtyInput.dataset.held) : null;
    const limit = qtyLimit(sideValue(), held);
    if (limit === null) qtyInput.removeAttribute("max");
    else qtyInput.max = String(limit);
    qtyInput.setCustomValidity("");
  };
  sideInputs.forEach((r) => r.addEventListener("change", syncLimit));
  syncLimit();
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const side = sideValue();
    const mode = (String(data.get("mode") ?? "long") as "long" | "trade");
    const qty = Number(data.get("qty"));
    const price = Number(data.get("price"));
    const at = String(data.get("at") ?? "").trim();
    if (!(qty > 0) || !(price > 0)) {
      if (status) status.textContent = "Quantity and price must be positive.";
      return;
    }
    if (status) status.textContent = "Recording…";
    try {
      const res = await api.recordTrade(symbol, side, qty, price, at || undefined, mode);
      if (status) {
        status.textContent = `Recorded. Cash ${Math.round(res.cash)}, realised ${Math.round(res.realised)}.`;
      }
      await route();
    } catch (e) {
      if (status) status.textContent = String(e);
    }
  });
}

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

/** A decision row opens to reveal the whole message; Approve opens it straight onto the form. */
function wireDecisions(): void {
  document.querySelectorAll<HTMLTableRowElement>(".decision-row").forEach((row) => {
    const id = row.dataset.decision;
    const detail = () => document.querySelector<HTMLTableRowElement>(`#decision-${id}`);
    const toggle = () => {
      const el = detail();
      if (!el) return;
      const open = !el.hidden;
      el.hidden = open;
      row.setAttribute("aria-expanded", String(!open));
      const label = row.querySelector("td.right button[data-open]");
      if (label) label.textContent = open ? "Open" : "Close";
    };
    row.querySelector<HTMLButtonElement>("[data-open]")?.addEventListener("click", (ev) => {
      ev.stopPropagation();
      toggle();
    });
    row.querySelector<HTMLButtonElement>("[data-approve-row]")?.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const el = detail();
      if (el?.hidden) toggle();
      const signal = row.dataset.signal;
      if (!signal) return;
      const form = document.querySelector<HTMLFormElement>(`[data-approve-form="${signal}"]`);
      if (form) {
        form.hidden = false;
        form.querySelector<HTMLInputElement>('input[name="qty"]')?.focus();
      }
    });
    // Clicking the row itself (but not a button) toggles it.
    row.addEventListener("click", (ev) => {
      if ((ev.target as HTMLElement).closest("button")) return;
      toggle();
    });
    row.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        toggle();
      }
    });
  });
}

function wireTargetProfit(symbol: string): void {
  const form = document.querySelector<HTMLFormElement>(".target-form");
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const raw = String(new FormData(form).get("pct") ?? "").trim();
    const pct = raw === "" ? null : Number(raw);
    if (pct !== null && (!Number.isFinite(pct) || pct <= 0)) {
      window.alert("Enter a positive percentage, or leave it blank.");
      return;
    }
    try {
      await api.setTargetProfit(symbol, pct);
      await route();
    } catch (e) {
      window.alert(String(e));
    }
  });
}

/** Opening a call fetches one row's bodies and renders the exchange in place. */
function wireCallRows(): void {
  // ?call=12 opens that row directly, so a specific exchange can be linked to.
  const wanted = new URLSearchParams(location.search).get("call");
  document.querySelectorAll<HTMLButtonElement>(".view-call").forEach((button) => {
    button.addEventListener("click", async () => {
      const id = Number(button.dataset.id);
      const cell = document.querySelector<HTMLTableCellElement>(`#call-${id} td`);
      const row = document.querySelector<HTMLTableRowElement>(`#call-${id}`);
      if (!cell || !row) return;
      const open = !row.hidden;
      if (open) {
        row.hidden = true;
        button.setAttribute("aria-expanded", "false");
        return;
      }
      row.hidden = false;
      button.setAttribute("aria-expanded", "true");
      cell.innerHTML = `<p class="empty">Reading the stored exchange…</p>`;
      try {
        cell.innerHTML = renderExchangeDetail(await api.callDetail(id));
      } catch (e) {
        cell.innerHTML = `<p class="empty">Could not read call ${id}: ${String(e)}</p>`;
      }
    });
    if (wanted && Number(wanted) === Number(button.dataset.id)) button.click();
  });
}

function wireSettings(tab: Tab, s: Settings): void {
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
        if (shouldDisableSave(budget, s.committed)) {
          if (status) status.textContent = `Committed is ${s.committed} — the budget cannot go below it.`;
          return;
        }
        patch = { budget, trading_budget: num("trading_budget"), currency: text("currency") };
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
          max_silence_secs: num("max_silence_secs"),
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
    await route();
  });

  document.querySelector<HTMLButtonElement>("#revert")?.addEventListener("click", async () => {
    clearDraft(tab);
    await route();
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

route();
window.setInterval(() => {
  if (window.location.pathname === "/") void route();
}, 60_000);
