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
import { wireMarket } from "./wiring/market";

let host: HTMLElement | null = null;
let lastPath = "/";
let dashboardEtag: string | null = null;

function failure(e: unknown): string {
  return `<div class="shell"><div class="error-band">Something failed: ${String(e)}
    <button class="btn" onclick="location.reload()">Retry</button></div></div>`;
}

/** Re-draw the page currently on screen. Views call this after an action; the timer calls it. */
export async function refresh(): Promise<void> {
  if (host) await draw(host, lastPath);
}

/**
 * The last of the plain renderers. Each stage of the Vue migration replaces one page inside here
 * with a real view; when the last one is gone, this module and `pages/` are deleted together.
 */
export async function draw(into: HTMLElement, pathWithQuery: string): Promise<void> {
  host = into;
  lastPath = pathWithQuery;
  const url = new URL(pathWithQuery, window.location.origin);
  const path = url.pathname;
  try {
    if (path === "/settings" || path.startsWith("/settings/")) {
      const wanted = path.startsWith("/settings/")
        ? decodeURIComponent(path.slice("/settings/".length))
        : "money";
      const tab: Tab = isTab(wanted) ? wanted : "money";
      if (tab !== wanted) history.replaceState(null, "", `/settings/${tab}`);
      const s = await api.settings();
      into.innerHTML = renderSettings(s, tab);
      wireSettings(tab, s, refresh);
    } else if (path === "/archived") {
      into.innerHTML = renderArchived(await api.archived());
      wireArchivedRows();
    } else if (path === "/calls") {
      const params = new URLSearchParams(location.search);
      const limit = [25, 50, 100].includes(Number(params.get("limit")))
        ? Number(params.get("limit"))
        : 25;
      const offset = Math.max(0, Number(params.get("offset") ?? 0) || 0);
      const filters = {
        prompt: params.get("prompt") ?? undefined,
        channel: params.get("channel") ?? undefined,
        model: params.get("model") ?? undefined,
        review: params.get("review") ?? undefined,
      };
      const [schedule, facets, list] = await Promise.all([
        api.schedule(),
        api.callFacets(),
        api.calls(offset, limit, filters),
      ]);
      into.innerHTML = renderCalls(
        schedule,
        0,
        list.rows,
        facets,
        filters,
        list.offset,
        list.limit,
        list.total,
      );
      wireRuns(refresh);
      // A filter starts the record over: page 2 of the old filter is not page 2 of the new one.
      document.querySelectorAll<HTMLSelectElement>("[data-filter]").forEach((select) => {
        select.addEventListener("change", () => {
          const next = new URLSearchParams(location.search);
          const name = select.dataset.filter!;
          if (select.value) next.set(name, select.value);
          else next.delete(name);
          next.delete("offset");
          window.location.href = `/calls?${next}`;
        });
      });
      // A different page size is the same view, re-cut from the top.
      document.querySelector<HTMLSelectElement>("[data-page-size]")?.addEventListener("change", (ev) => {
        const size = (ev.target as HTMLSelectElement).value;
        window.location.href = `/calls?offset=0&limit=${size}`;
      });
      wireCallRows(refresh);
    } else if (path.startsWith("/i/")) {
      const symbol = decodeURIComponent(path.slice(3));
      // One request: the analysis payload carries the instrument, its signals and its fills.
      const analysis = await api.analysis(symbol);
      into.innerHTML = renderInstrument(analysis, analysis, analysis.trades);
      wireTradeForm(symbol, refresh);
      wireFillEdits(refresh);
      wireDecisions();
      wireDecisionActions(refresh);
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
      into.innerHTML = renderLedger(d);
      // The nav belongs inside the content column, not floating in the page margin.
      wireRows();
      wireAddDialog();
      wireDecisionActions(refresh);
    }
  } catch (e) {
    into.innerHTML = failure(e);
  }
  // The rail's market chip is filled after the draw: no page waits on it.
  void wireMarket();
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
      if (status) {
        status.textContent = r.cached
          ? `Stored ${r.bars} bars and ${r.headlines} headlines.`
          : `Fetched ${r.bars} bars — archived, so nothing was stored.`;
      }
      refresh();
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
      refresh();
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
  const original = button.textContent ?? "Archive";
  let armed = false;
  button.addEventListener("click", async () => {
    if (!armed) {
      armed = true;
      button.textContent = "Archive? Click again — prices cleared, fills kept";
      if (status) status.textContent = "Archiving clears prices and headlines; fills and decisions stay.";
      window.setTimeout(() => {
        if (armed) {
          armed = false;
          button.textContent = original;
        }
      }, 5000);
      return;
    }
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
      refresh();
    } catch (e) {
      window.alert(String(e));
    }
  });
}

/**
 * The page refreshes on a minute timer. A page you are typing into does not, because re-rendering
 * would throw the form away. Row state that must survive a redraw is kept in `openCalls`.
 */

export function startAutoRefresh(): void {
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
  refresh();
  if (path === "/calls" && document.querySelector(".runs-strip .pill.running")) {
    window.setTimeout(() => {
      const el = document.activeElement;
      const busy =
        el instanceof HTMLInputElement ||
        el instanceof HTMLTextAreaElement ||
        el instanceof HTMLSelectElement;
      if (!busy) refresh();
    }, 4_000);
  }
}, 60_000);
}
