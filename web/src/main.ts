import { api, dashboardCached, type Hit, type Settings } from "./api";
import { renderCalls, renderExchangeDetail } from "./pages/calls";
import { renderArchived } from "./pages/archived";
import { qtyLimit, renderInstrument } from "./pages/instrument";
import { renderLedger } from "./pages/ledger";
import { escapeHtml } from "./format";
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
    } else if (path === "/archived") {
      app.innerHTML = renderArchived(await api.archived());
      wireArchivedRows();
    } else if (path === "/calls") {
      const params = new URLSearchParams(location.search);
      const limit = [25, 50, 100].includes(Number(params.get("limit")))
        ? Number(params.get("limit"))
        : 25;
      const offset = Math.max(0, Number(params.get("offset") ?? 0) || 0);
      const list = await api.calls(offset, limit);
      app.innerHTML = renderCalls(list.rows, list.offset, list.limit, list.total);
      // A different page size is the same view, re-cut from the top.
      document.querySelector<HTMLSelectElement>("[data-page-size]")?.addEventListener("change", (ev) => {
        const size = (ev.target as HTMLSelectElement).value;
        window.location.href = `/calls?offset=0&limit=${size}`;
      });
      wireCallRows();
    } else if (path.startsWith("/i/")) {
      const symbol = decodeURIComponent(path.slice(3));
      // One request: the analysis payload carries the instrument, its signals and its fills.
      const analysis = await api.analysis(symbol);
      app.innerHTML = renderInstrument(analysis, analysis, analysis.trades);
      wireTradeForm(symbol);
      wireFillEdits();
      wireDecisions();
      wireDecisionActions();
      wireDeleteInstrument();
      wireRefreshInstrument();
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
      app.innerHTML = renderLedger(d);
      // The nav belongs inside the content column, not floating in the page margin.
      wireRows();
      wireAddDialog();
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

/**
 * Adding an instrument: one dialog, opened from the header (or ⌘K / Ctrl-K), results grouped by
 * class and chosen with ↑↓ ⏎. The dialog is the only place a search happens, so there is never a
 * bare input sitting at the bottom of a page.
 */
function wireAddDialog(): void {
  const dialog = document.querySelector<HTMLDialogElement>("#add-dialog");
  const body = document.querySelector<HTMLElement>(".add-dialog-body");
  if (!dialog || !body) return;
  if (!dialog.contains(body)) dialog.append(body);
  body.hidden = false;

  const input = dialog.querySelector<HTMLInputElement>("#search")!;
  const box = dialog.querySelector<HTMLDivElement>("#results")!;
  let timer: number | undefined;
  let hits: Hit[] = [];
  let active = 0;

  // Six is a chooser, not a directory: it keeps the list on one screen, so no scrollbar can
  // push the rows out of line with the field above them.
  const SHOWN = 6;
  const groups = (list: Hit[]) => {
    const stock = list.filter((h) => h.class !== "mutual_fund");
    const fund = list.filter((h) => h.class === "mutual_fund");
    return [
      { label: "Stocks", items: stock },
      { label: "Mutual funds", items: fund },
    ].filter((g) => g.items.length);
  };

  const draw = () => {
    const parts: string[] = [];
    let index = 0;
    for (const g of groups(hits.slice(0, SHOWN))) {
      parts.push(`<div class="dlg-group">${g.label}</div>`);
      for (const h of g.items) {
        const i = index++;
        parts.push(`<div class="dlg-opt ${i === active ? "sel" : ""}" role="option"
          aria-selected="${i === active}" data-i="${i}" tabindex="-1">
          <b>${escapeHtml(h.name)}</b>
          <span class="meta">${escapeHtml(h.code)} · ${h.class === "mutual_fund" ? "fund" : "stock"}</span>
        </div>`);
      }
    }
    box.innerHTML = parts.length
      ? `${parts.join("")}${
          hits.length > SHOWN
            ? `<p class="dlg-more">${hits.length - SHOWN} more — keep typing to narrow it</p>`
            : ""
        }`
      : input.value.trim().length >= 2
        ? `<p class="empty">Nothing matches that.</p>`
        : `<p class="empty">Type a name, a ticker, or an AMFI code.</p>`;
    box.querySelectorAll<HTMLElement>(".dlg-opt").forEach((el) => {
      el.addEventListener("click", () => {
        active = Number(el.dataset.i);
        draw();
        void add();
      });
    });
  };

  const add = async () => {
    const hit = hits[active];
    if (!hit) return;
    const row = box.querySelector<HTMLElement>(`.dlg-opt[data-i="${active}"]`);
    if (row) row.textContent = `Adding ${hit.code}…`;
    try {
      await api.addInstrument(hit.code, hit.name, hit.class);
      window.location.href = `/i/${encodeURIComponent(hit.code)}`;
    } catch (e) {
      if (row) row.textContent = `Failed: ${String(e)}`;
    }
  };

  const open = () => {
    if (!dialog.open) dialog.showModal();
    input.value = "";
    hits = [];
    active = 0;
    draw();
    input.focus();
  };

  document.querySelectorAll<HTMLElement>("[data-add-open]").forEach((b) => {
    b.addEventListener("click", open);
  });
  document.addEventListener("keydown", (ev) => {
    if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "k") {
      ev.preventDefault();
      open();
    }
  });
  dialog.addEventListener("click", (ev) => {
    if (ev.target === dialog) dialog.close(); // the backdrop
  });
  dialog.addEventListener("close", () => input.blur());

  input.addEventListener("input", () => {
    window.clearTimeout(timer);
    const q = input.value.trim();
    if (q.length < 2) {
      hits = [];
      active = 0;
      draw();
      return;
    }
    timer = window.setTimeout(async () => {
      try {
        hits = await api.search(q);
        active = 0;
        draw();
      } catch {
        box.innerHTML = `<p class="empty">Search unavailable.</p>`;
      }
    }, 250);
  });

  input.addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
      ev.preventDefault();
      const count = Math.min(hits.length, 6);
      if (!count) return;
      active = (active + (ev.key === "ArrowDown" ? 1 : -1) + count) % count;
      draw();
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      void add();
    }
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
        // A plan posts one fill per leg; a single decision posts its own.
        const plan = form.dataset.plan;
        const fills = plan
          ? plan.split(",").map((leg) => ({
              id: Number(leg),
              qty: Number(form.querySelector<HTMLInputElement>(`[name="qty-${leg}"]`)?.value),
              price: Number(form.querySelector<HTMLInputElement>(`[name="price-${leg}"]`)?.value),
            }))
          : undefined;
        await api.approveSignal(id, {
          qty: fills ? undefined : Number(field(String(id), "qty")?.value),
          price: fills ? undefined : Number(field(String(id), "price")?.value),
          at: field(String(id), "at")?.value || undefined,
          fills,
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

/**
 * Fills are correctable. The panel opens in place of the row it corrects, so you can see the
 * number you are changing next to the one you are changing it to.
 */
function wireFillEdits(): void {
  const panel = document.querySelector<HTMLElement>("#record-panel");
  document.querySelectorAll<HTMLButtonElement>("[data-record-toggle]").forEach((button) => {
    button.addEventListener("click", () => {
      if (!panel) return;
      panel.hidden = !panel.hidden;
      button.setAttribute("aria-expanded", String(!panel.hidden));
      if (!panel.hidden) panel.querySelector<HTMLInputElement>('input[name="qty"]')?.focus();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-edit-fill]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.editFill!;
      const row = document.querySelector<HTMLElement>(`[data-fill-edit="${id}"]`);
      if (!row) return;
      row.hidden = !row.hidden;
      button.setAttribute("aria-expanded", String(!row.hidden));
      if (!row.hidden) row.querySelector<HTMLInputElement>('input[name="qty"]')?.focus();
    });
  });
  document.querySelectorAll<HTMLButtonElement>("[data-cancel-fill]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.cancelFill!;
      const row = document.querySelector<HTMLElement>(`[data-fill-edit="${id}"]`);
      if (row) row.hidden = true;
    });
  });

  document
    .querySelectorAll<HTMLFormElement>("[data-edit-form]")
    .forEach((form) => {
      form.addEventListener("submit", async (ev) => {
        ev.preventDefault();
        const id = Number(form.dataset.editForm);
        const status = document.querySelector<HTMLElement>(`[data-fill-status="${id}"]`);
        const value = (name: string) =>
          form.querySelector<HTMLInputElement>(`[name="${name}"]`)?.value ?? "";
        if (status) status.textContent = "Saving…";
        try {
          await api.editTrade(id, {
            qty: Number(value("qty")),
            price: Number(value("price")),
            at: value("at") || undefined,
          });
          await route();
        } catch (e) {
          if (status) status.textContent = String(e);
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

/** An archived row opens the instrument: its record is there, its price history is not. */
function wireArchivedRows(): void {
  document.querySelectorAll<HTMLTableRowElement>('.blotter .row').forEach((row) => {
    const symbol = row.dataset.symbol;
    if (!symbol) return;
    const go = () => {
      location.href = `/i/${encodeURIComponent(symbol)}`;
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

/** Fetch this symbol now; then re-render so the chart and decisions match. */
function wireRefreshInstrument(): void {
  const button = document.querySelector<HTMLButtonElement>("#refresh-instrument");
  if (!button) return;
  const status = document.querySelector<HTMLElement>("#refresh-status");
  const symbol = button.dataset.symbol ?? "";
  button.addEventListener("click", async () => {
    button.disabled = true;
    if (status) status.textContent = "Fetching…";
    try {
      const r = await api.refreshInstrument(symbol);
      if (status) status.textContent = `Stored ${r.bars} bars and ${r.news} headlines.`;
      await route();
    } catch (e) {
      if (status) status.textContent = String(e);
      button.disabled = false;
    }
  });
}

/** Delete or archive, whichever the server decides — and say which it did. */
function wireDeleteInstrument(): void {
  const button = document.querySelector<HTMLButtonElement>("#archive-instrument");
  if (!button) return;
  const status = document.querySelector<HTMLElement>("#delete-status");
  const symbol = button.dataset.symbol ?? "";
  if (button.dataset.held && Number(button.dataset.held) > 0) {
    button.disabled = true;
    return;
  }
  button.addEventListener("click", async () => {
    if (!window.confirm(`Archive ${symbol}? Prices and headlines are cleared; fills and decisions are kept.`)) return;
    button.disabled = true;
    if (status) status.textContent = "Removing…";
    try {
      const r = await api.archiveInstrument(symbol);
      if (status) {
        status.textContent = r.archived
          ? "Archived. Its record is on the Archived page; its prices were cleared."
          : "Could not archive.";
      }
      window.setTimeout(() => {
        location.href = "/";
      }, 900);
    } catch (e) {
      if (status) status.textContent = String(e);
      button.disabled = false;
    }
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

/**
 * Keep every page current. Data pages refresh on a timer; a page you are typing into does not,
 * because re-rendering would throw away the form.
 */
window.setInterval(() => {
  const path = window.location.pathname;
  const dataPage =
    path === "/" || path.startsWith("/i/") || path === "/calls" || path === "/archived";
  if (!dataPage) return;
  const el = document.activeElement;
  const typing =
    el instanceof HTMLInputElement ||
    el instanceof HTMLTextAreaElement ||
    el instanceof HTMLSelectElement;
  const draftOpen = !!document.querySelector("form.approve-form:not([hidden]), form.reject-form:not([hidden])");
  if (typing || draftOpen) return;
  void route();
}, 60_000);
