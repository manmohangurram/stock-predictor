import { api, dashboardCached, type Settings } from "./api";
import { renderCalls } from "./pages/calls";
import { renderArchived } from "./pages/archived";
import { renderInstrument } from "./pages/instrument";
import { renderLedger } from "./pages/ledger";
import { isTab, renderSettings, type Tab } from "./pages/settings";
import { wireSettings } from "./wiring/settings";
import { wireAddDialog } from "./wiring/add-instrument";
import { wireRuns } from "./wiring/runs";
import { wireFillEdits, wireTradeForm } from "./wiring/fills";
import { wireCallRows } from "./wiring/calls";
import { wireDecisionActions } from "./wiring/decisions";

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
      wireSettings(tab, s, route);
    } else if (path === "/archived") {
      app.innerHTML = renderArchived(await api.archived());
      wireArchivedRows();
    } else if (path === "/calls") {
      const params = new URLSearchParams(location.search);
      const limit = [25, 50, 100].includes(Number(params.get("limit")))
        ? Number(params.get("limit"))
        : 25;
      const offset = Math.max(0, Number(params.get("offset") ?? 0) || 0);
      const [runs, list] = await Promise.all([api.runs(25), api.calls(offset, limit)]);
      app.innerHTML = renderCalls(runs.runs, runs.running, list.rows, list.offset, list.limit, list.total);
      wireRuns(route);
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
      wireTradeForm(symbol, route);
      wireFillEdits(route);
      wireDecisions();
      wireDecisionActions(route);
      wireDeleteInstrument();
      wireRestoreInstrument();
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
      wireDecisionActions(route);
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
/** Restore an archived instrument from its own page: it goes back in the book, prices return. */
function wireRestoreInstrument(): void {
  const button = document.querySelector<HTMLButtonElement>("#restore-instrument");
  if (!button) return;
  const status = document.querySelector<HTMLElement>("#delete-status");
  button.addEventListener("click", async () => {
    button.disabled = true;
    if (status) status.textContent = "Restoring…";
    try {
      await api.restoreInstrument(button.dataset.symbol ?? "");
      await route();
    } catch (e) {
      button.disabled = false;
      if (status) status.textContent = String(e);
    }
  });
}

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
/// Rows the reader has open. A refresh must not close them — that is the whole reason this exists.
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
  // A running job is the one thing worth watching closely: check again in a moment, so the page
  // shows the result as it lands instead of up to a minute later.
  if (path === "/calls" && document.querySelector(".runs-strip .pill.running")) {
    window.setTimeout(() => {
      const el = document.activeElement;
      const busy =
        el instanceof HTMLInputElement ||
        el instanceof HTMLTextAreaElement ||
        el instanceof HTMLSelectElement;
      if (!busy) void route();
    }, 4_000);
  }
}, 60_000);
