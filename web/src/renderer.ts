import { api, dashboardCached, type Settings } from "./api";
import { renderArchived } from "./pages/archived";
import { isTab, renderSettings, type Tab } from "./pages/settings";
import { wireSettings } from "./wiring/settings";

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
    }
  } catch (e) {
    into.innerHTML = failure(e);
  }
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
